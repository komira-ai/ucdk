// SPDX-License-Identifier: Apache-2.0
//! Who is running: the run id, the attempt and the context a driver supplies, checked.
//!
//! - The run id is 1 to 63 bytes of `[a-z0-9_-]`, so the same id can later be stamped on a
//!   cloud object as a label. ucdk mints none: a driver passes one.
//! - The attempt is a positive number with no default. An omitted attempt is a usage error,
//!   never attempt 1.
//! - The context is at most 32 pairs. A key is `[a-z][a-z0-9_]{0,31}`; a value is at most 256
//!   bytes of printable ASCII. It is recorded verbatim and trusted by nothing: anyone who runs
//!   ucdk can write anything in it, so no decision reads it and no hash covers it.
//!
//! ucdk reads no CI vendor's environment variable for any of this.

use std::collections::BTreeMap;
use std::fmt;

use ucdk_release_proto::v1::RunIdentity;

/// The most bytes of a run id.
pub const RUN_ID_MAX_BYTES: usize = 63;
/// The most context pairs.
pub const CONTEXT_MAX_ENTRIES: usize = 32;
/// The most bytes of a context key.
pub const CONTEXT_KEY_MAX_BYTES: usize = 32;
/// The most bytes of a context value.
pub const CONTEXT_VALUE_MAX_BYTES: usize = 256;

/// Why a run identity is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunIdentityError {
    /// The run id is empty.
    RunIdEmpty,
    /// The run id is longer than [`RUN_ID_MAX_BYTES`].
    RunIdTooLong { bytes: usize },
    /// The run id holds a byte outside `[a-z0-9_-]`.
    RunIdByte { at: usize },
    /// No attempt was given (a message reads an unset number as 0).
    AttemptMissing,
    /// The attempt text is not a positive decimal number without sign or leading zero, or does
    /// not fit.
    AttemptText { text: String },
    /// More than [`CONTEXT_MAX_ENTRIES`] pairs.
    ContextTooMany { count: usize },
    /// A `--context` argument without an `=`.
    ContextNotKeyValue { arg: String },
    /// A context key outside its grammar.
    ContextKey { key: String },
    /// A context key given twice.
    ContextKeyTwice { key: String },
    /// A context value that is too long.
    ContextValueTooLong { key: String, bytes: usize },
    /// A context value holding a byte outside printable ASCII.
    ContextValueByte { key: String, at: usize },
}

/// Checks the run id, the attempt and the context of `identity` against the grammar above.
pub fn check(identity: &RunIdentity) -> Result<(), RunIdentityError> {
    check_run_id(&identity.run_id)?;
    if identity.attempt == 0 {
        return Err(RunIdentityError::AttemptMissing);
    }
    if identity.context.len() > CONTEXT_MAX_ENTRIES {
        return Err(RunIdentityError::ContextTooMany {
            count: identity.context.len(),
        });
    }
    identity
        .context
        .iter()
        .try_for_each(|(key, value)| check_pair(key, value))
}

fn check_run_id(run_id: &str) -> Result<(), RunIdentityError> {
    let bytes = run_id.as_bytes();
    if bytes.is_empty() {
        return Err(RunIdentityError::RunIdEmpty);
    }
    if bytes.len() > RUN_ID_MAX_BYTES {
        return Err(RunIdentityError::RunIdTooLong { bytes: bytes.len() });
    }
    match bytes
        .iter()
        .position(|b| !matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
    {
        Some(at) => Err(RunIdentityError::RunIdByte { at }),
        None => Ok(()),
    }
}

fn check_pair(key: &str, value: &str) -> Result<(), RunIdentityError> {
    let word = key.bytes().enumerate().all(|(at, b)| match b {
        b'a'..=b'z' => true,
        b'0'..=b'9' | b'_' => at > 0,
        _ => false,
    });
    if key.is_empty() || key.len() > CONTEXT_KEY_MAX_BYTES || !word {
        return Err(RunIdentityError::ContextKey {
            key: key.to_owned(),
        });
    }
    if value.len() > CONTEXT_VALUE_MAX_BYTES {
        return Err(RunIdentityError::ContextValueTooLong {
            key: key.to_owned(),
            bytes: value.len(),
        });
    }
    match value.bytes().position(|b| !(0x20..=0x7e).contains(&b)) {
        Some(at) => Err(RunIdentityError::ContextValueByte {
            key: key.to_owned(),
            at,
        }),
        None => Ok(()),
    }
}

/// Reads the text of `--attempt`: decimal digits only, no sign, no leading zero, at least 1
/// and at most `u32::MAX`.
pub fn parse_attempt(text: &str) -> Result<u32, RunIdentityError> {
    let refused = || RunIdentityError::AttemptText {
        text: text.to_owned(),
    };
    if text.is_empty() || text.starts_with('0') || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(refused());
    }
    text.parse().map_err(|_| refused())
}

/// Builds the identity of `--run-id`, `--attempt` and every `--context key=value`, checked.
/// An absent attempt is refused, never defaulted. The first `=` of a context argument splits
/// it; a key given twice is refused, not overwritten. The first fault is the run id's, then the
/// attempt's, then the context's.
pub fn from_flags(
    run_id: &str,
    attempt: Option<&str>,
    context: &[String],
) -> Result<RunIdentity, RunIdentityError> {
    check_run_id(run_id)?;
    let attempt = parse_attempt(attempt.ok_or(RunIdentityError::AttemptMissing)?)?;
    if context.len() > CONTEXT_MAX_ENTRIES {
        return Err(RunIdentityError::ContextTooMany {
            count: context.len(),
        });
    }
    let mut pairs = BTreeMap::new();
    for arg in context {
        let (key, value) = arg
            .split_once('=')
            .ok_or_else(|| RunIdentityError::ContextNotKeyValue { arg: arg.clone() })?;
        check_pair(key, value)?;
        if pairs.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(RunIdentityError::ContextKeyTwice {
                key: key.to_owned(),
            });
        }
    }
    Ok(RunIdentity {
        run_id: run_id.to_owned(),
        attempt,
        context: pairs,
    })
}

/// Whether two identities name the same run: the same id, the same attempt and the same
/// context. The context is part of the comparison and of no hash.
pub fn same_as(a: &RunIdentity, b: &RunIdentity) -> bool {
    a == b
}

impl fmt::Display for RunIdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RunIdEmpty => write!(f, "the run id is empty"),
            Self::RunIdTooLong { bytes } => {
                write!(f, "the run id is {bytes} bytes; at most {RUN_ID_MAX_BYTES}")
            }
            Self::RunIdByte { at } => write!(
                f,
                "the run id holds a byte outside [a-z0-9_-] at {at} (label-safe ids only)"
            ),
            Self::AttemptMissing => write!(f, "no attempt was given; it has no default"),
            Self::AttemptText { text } => {
                write!(f, "attempt '{text}' is not a positive decimal number")
            }
            Self::ContextTooMany { count } => {
                write!(f, "{count} context pairs; at most {CONTEXT_MAX_ENTRIES}")
            }
            Self::ContextNotKeyValue { arg } => {
                write!(f, "context '{arg}' is not key=value")
            }
            Self::ContextKey { key } => write!(
                f,
                "context key '{key}' is not [a-z][a-z0-9_]* of at most {CONTEXT_KEY_MAX_BYTES} bytes"
            ),
            Self::ContextKeyTwice { key } => write!(f, "context key '{key}' is given twice"),
            Self::ContextValueTooLong { key, bytes } => write!(
                f,
                "context {key}: the value is {bytes} bytes; at most {CONTEXT_VALUE_MAX_BYTES}"
            ),
            Self::ContextValueByte { key, at } => write!(
                f,
                "context {key}: the value holds a byte outside printable ASCII at {at}"
            ),
        }
    }
}

impl std::error::Error for RunIdentityError {}
