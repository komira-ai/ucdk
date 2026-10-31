// SPDX-License-Identifier: Apache-2.0
//! The classification of mopm's answers into [`SeamError`]s: one table, as functions.
//!
//! The code of an answer is `error.code` of mopm's body; the HTTP status alone decides only where
//! there is no code to read (authorisation, rate limits, a server failure). `Transient` means
//! "failed, and it may have landed" in the seam, so it is used wherever an effect might exist;
//! `Permanent` and `Refused` are never used after a request that may have landed. No message of
//! mopm is parsed, and none is copied into an error: only the status and the code are.

use std::time::Duration;

use serde_json::Value;
use ucdk_release::codec::GrantFault;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Backoff, SeamError};

use crate::transport::{Response, TransportError};

/// The wait, in seconds, when a 429 carries no usable `Retry-After`: small and fixed, so that a
/// registry that sends none is never retried in a storm and never waited on for long.
pub const DEFAULT_RETRY_AFTER_SECONDS: u64 = 5;

/// The request an answer is to. The table differs by what the request could have done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// A `GET` of a package, a version, the versions, a search or the index, or of a file.
    Read,
    /// `POST /groups`: reserves numbers and returns upload URLs. Nothing is visible yet.
    Create,
    /// `POST /groups/{id}/commit`: from its first byte written, an effect may have landed.
    Commit,
    /// `GET /groups/{id}` after the commit was sent.
    Poll,
    /// `PUT /packages/{n}/{v}/yank`.
    Yank,
}

impl Op {
    /// Whether a request of this kind may have changed the registry once it was sent, so that a
    /// failure with no clear refusal is `Unknown` and never `Permanent`.
    fn may_have_landed(self) -> bool {
        matches!(self, Op::Commit | Op::Poll | Op::Yank)
    }
}

/// What an answer means to the verb that sent the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mapped {
    /// The package or version is genuinely absent: `Absent` for `observe`, `None` for `fetch`,
    /// an empty list for `newest`. Only a parsed 404 `package_not_found` or `version_not_found`
    /// on a read; any other failure of a read is never this.
    Missing,
    /// 409 `version_exists` on create: the version is held, yanked or burned. The caller reads
    /// the version record to decide `Duplicate` or the sealed-version refusal.
    Duplicate,
    /// 409 `group_commit_refused`: the caller reads the status of the group and decides.
    CommitRefused,
    /// 409 `group_conflict` on create, with the reason mopm's closed list gives. The class of a
    /// reason is [`ConflictReason::answer`]; some classes need one more read, so the caller
    /// finishes them.
    Conflict(ConflictReason),
    Failed(SeamError),
}

/// Why mopm refused a create with `group_conflict`: the `error.reason` field, a closed list in
/// mopm's API description. The message is free text and is never read. A registry that predates
/// the field, a reason not on the list and a value that is not text are all [`Unlisted`], which a
/// client must treat as an absent reason.
///
/// [`Unlisted`]: ConflictReason::Unlisted
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReason {
    /// A live group holds a number of the set: another account's, or one of the caller's own
    /// that is validating or committed (never superseded).
    NumberHeld,
    /// The group failed: its numbers are burned.
    GroupFailed,
    /// The caller has the most open groups it may have.
    OpenCap,
    /// The create would end more of the caller's own groups than one create may.
    SupersedeCap,
    /// A reserved row that is not yet free blocks the number.
    TakeoverBlocked,
    /// Another publish took a number while the group was being created; the group was ended.
    NumberTaken,
    /// `on_overlap: refuse` met an overlapping open group of the caller's own.
    OverlapRefused,
    /// No reason, or one this crate does not know.
    Unlisted,
}

/// What a [`ConflictReason`] means to `stage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictAnswer {
    /// The numbers are held by a group that stands: `Duplicate`.
    Duplicate,
    /// `Duplicate` if the version record now shows the coordinate held, else `Transient`: the
    /// caller reads the record.
    DuplicateIfHeld,
    /// An error that settles the call.
    Fail(SeamError),
}

impl ConflictReason {
    /// The reason in `value`, an `error.reason` already read from the body.
    pub fn parse(value: Option<&str>) -> Self {
        match value {
            Some("number_held") => Self::NumberHeld,
            Some("group_failed") => Self::GroupFailed,
            Some("open_cap") => Self::OpenCap,
            Some("supersede_cap") => Self::SupersedeCap,
            Some("takeover_blocked") => Self::TakeoverBlocked,
            Some("number_taken") => Self::NumberTaken,
            Some("overlap_refused") => Self::OverlapRefused,
            _ => Self::Unlisted,
        }
    }

    /// The class of the reason. A caller over its open-group cap is never told `Duplicate`, so
    /// the engine cannot loop on it; a failed group is `Transient` (the burn is an effect, and
    /// the engine's observe reads it as burned); an unknown reason is `Unknown`, because nothing
    /// says what it means.
    pub fn answer(self) -> ConflictAnswer {
        let fail = ConflictAnswer::Fail;
        match self {
            Self::NumberHeld | Self::OverlapRefused => ConflictAnswer::Duplicate,
            Self::NumberTaken => ConflictAnswer::DuplicateIfHeld,
            Self::OpenCap | Self::SupersedeCap => fail(SeamError::Refused(Reason::Channel)),
            Self::GroupFailed => fail(SeamError::Transient(
                "mopm answered 409 group_conflict: the numbers are burned".to_owned(),
            )),
            Self::TakeoverBlocked => fail(SeamError::Transient(
                "mopm answered 409 group_conflict: a number is not yet free".to_owned(),
            )),
            Self::Unlisted => fail(SeamError::Unknown(
                "mopm answered 409 group_conflict with no reason this client knows".to_owned(),
            )),
        }
    }
}

/// The `error.code` of mopm's error body, if the body is JSON and has one.
pub fn error_code(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let code = value.pointer("/error/code")?.as_str()?;
    Some(code.to_owned())
}

/// The `error.reason` of mopm's error body, if the body is JSON and has one that is text.
pub fn error_reason(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let reason = value.pointer("/error/reason")?.as_str()?;
    Some(reason.to_owned())
}

/// The seconds of a `Retry-After` that is a plain number; `None` for a missing header, an
/// HTTP date or anything else. A number too large for 64 bits is the largest one, so it is over
/// every ceiling.
fn retry_after_seconds(response: &Response) -> Option<u64> {
    let value = response.header("retry-after")?.trim();
    let digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
    digits.then(|| value.parse().unwrap_or(u64::MAX))
}

/// A 429: `RateLimited` for the seconds `Retry-After` states (rounded up to ticks by the seam,
/// at least one), never more than `ceiling`; a longer wait is a refusal, because the daily file
/// allowance can ask for up to a day and a `Pause` of that length is never the answer.
pub(crate) fn rate_limited(response: &Response, ceiling: Duration) -> SeamError {
    let seconds = retry_after_seconds(response).unwrap_or(DEFAULT_RETRY_AFTER_SECONDS);
    if Duration::from_secs(seconds) > ceiling {
        return SeamError::Refused(Reason::Channel);
    }
    SeamError::RateLimited(Backoff::after_millis(seconds.max(1).saturating_mul(1000)))
}

/// Classifies a failure answer of `op`. `None` for a success status (200 to 299), which is not
/// an error. `ceiling` is `Config::retry_after_ceiling`.
pub fn map_reply(op: Op, response: &Response, ceiling: Duration) -> Option<Mapped> {
    if response.is_success() {
        return None;
    }
    let status = response.status;
    let code = error_code(&response.body);
    let status_code = format!(
        "mopm answered {status} {}",
        code.as_deref().unwrap_or("with no error code")
    );
    let refuse = || Mapped::Failed(SeamError::Refused(Reason::Channel));
    let permanent = || Mapped::Failed(SeamError::Permanent(status_code.clone()));
    let transient = || Mapped::Failed(SeamError::Transient(status_code.clone()));
    let unknown = || Mapped::Failed(SeamError::Unknown(status_code.clone()));
    let denied = || Mapped::Failed(SeamError::GrantRefused(GrantFault::Denied));
    let rate = || Mapped::Failed(rate_limited(response, ceiling));
    Some(match (op, code.as_deref()) {
        // After the commit was sent, nothing a status read does not prove is ever `Permanent`.
        (Op::Poll, _) => unknown(),
        (_, _) if status == 429 => rate(),
        (_, Some("read_only_mode")) => refuse(),
        (_, Some("route_not_found")) => refuse(),
        (Op::Commit, Some("group_commit_refused")) => Mapped::CommitRefused,
        // A missing file: repeat the identical create, upload, commit.
        (Op::Commit, Some("upload_invalid")) => transient(),
        // Authorisation of a commit that was sent: the token may have been honoured first.
        (Op::Commit, _) => unknown(),
        (Op::Read, Some("package_not_found" | "version_not_found")) => Mapped::Missing,
        (Op::Create | Op::Yank, Some("package_not_found" | "not_owner")) => refuse(),
        (Op::Create, Some("limit_exceeded" | "payload_too_large")) => refuse(),
        (_, Some("token_expired")) if status == 401 => {
            Mapped::Failed(SeamError::GrantRefused(GrantFault::Expired))
        }
        (_, _) if status == 401 || status == 403 => denied(),
        (Op::Create, Some("version_exists")) => Mapped::Duplicate,
        (Op::Create, Some("group_conflict")) => Mapped::Conflict(ConflictReason::parse(
            error_reason(&response.body).as_deref(),
        )),
        (
            Op::Create,
            Some("invalid_name" | "invalid_version" | "invalid_request" | "upload_invalid"),
        ) => permanent(),
        (Op::Create, _) if status == 404 => permanent(),
        (Op::Yank, Some("invalid_transition" | "invalid_request" | "version_not_found")) => {
            permanent()
        }
        (Op::Read | Op::Create, _) => transient(),
        (Op::Yank, _) => unknown(),
    })
}

/// A failure to get any answer. A request that may have changed the registry is `Unknown`; a
/// read, and a create (which changes nothing visible), is `Transient`.
pub fn map_transport(op: Op, error: &TransportError) -> SeamError {
    if op.may_have_landed() {
        SeamError::Unknown(error.to_string())
    } else {
        SeamError::Transient(error.to_string())
    }
}

/// A well-formed answer that violates the response schema (a 200 without a `sha256`, a record
/// with two files for one subdirectory). `Permanent` where nothing could have landed; `Unknown`
/// where the request may have.
pub fn map_schema_violation(op: Op, what: &str) -> SeamError {
    if op.may_have_landed() {
        SeamError::Unknown(format!("malformed answer: {what}"))
    } else {
        SeamError::Permanent(format!("malformed answer: {what}"))
    }
}
