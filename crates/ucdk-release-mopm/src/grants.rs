// SPDX-License-Identifier: Apache-2.0
//! The grant envelope: what the mopm grant source writes into [`Grant::issued`] and the
//! publisher reads back, and the checks the publisher makes of a grant before it sends anything.
//!
//! The envelope is the grant's secret. It is length-framed text, so no field can run into the
//! next: the version, the phase, the exceptional mark, the registry location, the actor, the
//! number of coordinates and the five fields of each, and last the token mopm minted. It is
//! unauthenticated and private to this crate: the publisher trusts the grant source that built
//! it. An [`Envelope`] is neither `Debug` nor `Clone` and its token is overwritten when it is
//! dropped.
//!
//! Every check here is client-side and sends nothing: a refused grant is
//! `GrantRefused(Denied)` (the grant is not for this call) or `GrantRefused(Expired)`.

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::{Grant, SeamError};

const VERSION: &str = "1";

/// Bytes that are not an envelope this crate wrote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;

/// What a grant says it is for, and the token it carries.
pub struct Envelope {
    pub phase: GrantPhase,
    /// The grant was exchanged for the exceptional subject; carried so a log shows it.
    pub exceptional: bool,
    /// The location of the registry the grant is for, compared byte for byte.
    pub registry: String,
    /// The actor of the group id: `u:<id>` or `ci:gh-repo:<id>`.
    pub actor: String,
    /// Every coordinate the grant covers, by all five fields.
    pub coordinates: Vec<Coordinate>,
    token: Vec<u8>,
}

impl Drop for Envelope {
    fn drop(&mut self) {
        self.token.fill(0);
    }
}

fn put(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(bytes.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(bytes);
}

fn phase_word(phase: GrantPhase) -> &'static str {
    match phase {
        GrantPhase::Stage => "stage",
        GrantPhase::Commit => "commit",
        GrantPhase::Read => "read",
    }
}

/// The next framed text of `rest` and what follows it. The length is canonical decimal digits:
/// no sign, no leading zero, so one text has one encoding.
fn take<'a>(rest: &mut &'a [u8]) -> Result<&'a [u8], Malformed> {
    let colon = rest.iter().position(|b| *b == b':').ok_or(Malformed)?;
    let digits = std::str::from_utf8(&rest[..colon]).map_err(|_| Malformed)?;
    let len: usize = digits.parse().map_err(|_| Malformed)?;
    if digits != len.to_string() {
        return Err(Malformed);
    }
    let body = &rest[colon + 1..];
    if len > body.len() {
        return Err(Malformed);
    }
    let (text, after) = body.split_at(len);
    *rest = after;
    Ok(text)
}

fn take_str(rest: &mut &[u8]) -> Result<String, Malformed> {
    String::from_utf8(take(rest)?.to_vec()).map_err(|_| Malformed)
}

impl Envelope {
    pub fn new(
        phase: GrantPhase,
        exceptional: bool,
        registry: String,
        actor: String,
        coordinates: Vec<Coordinate>,
        token: Vec<u8>,
    ) -> Self {
        Self {
            phase,
            exceptional,
            registry,
            actor,
            coordinates,
            token,
        }
    }

    /// The token mopm minted, as sent in the `authorization` header. `None` for a Read
    /// envelope, which carries none.
    pub fn bearer_token(&self) -> Option<&[u8]> {
        (!self.token.is_empty()).then_some(self.token.as_slice())
    }

    /// The bytes of the grant's secret.
    pub fn encode(&self) -> Vec<u8> {
        Vec::new()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, Malformed> {
        let _ = bytes;
        Err(Malformed)
    }
}

/// The verb a grant is presented to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Observe,
    Fetch,
    Newest,
    List,
    Stage,
    Abort,
    Commit,
    Yank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Needs {
    Any,
    Stage,
    Commit,
}

impl Verb {
    fn is_read(self) -> bool {
        self.needs() == Needs::Any
    }

    /// The phase that may present a grant to this verb: a stage grant stages and aborts, a
    /// commit grant commits and yanks, and a read takes a grant of any phase (the group read of
    /// an observe needs a token, which a Read envelope does not carry; the caller sees that).
    fn needs(self) -> Needs {
        match self {
            Verb::Observe | Verb::Fetch | Verb::Newest | Verb::List => Needs::Any,
            Verb::Stage | Verb::Abort => Needs::Stage,
            Verb::Commit | Verb::Yank => Needs::Commit,
        }
    }
}

/// What a call is about to do with a grant.
pub struct Use<'a> {
    /// The configured registry location, compared byte for byte with the envelope's.
    pub registry: &'a str,
    /// The injected clock's now, in milliseconds.
    pub now_ms: u64,
    pub verb: Verb,
    /// Every coordinate the call names: for `observe` the key and the whole declared set, for
    /// `abort` the whole set, for a yank every held sibling. Empty for `newest` and `list`.
    pub coordinates: &'a [&'a Coordinate],
    /// How long the call may run: the grant must still be good this long after now.
    pub lasts_ms: u64,
}

fn denied() -> SeamError {
    SeamError::GrantRefused(GrantFault::Denied)
}

/// Checks `grant` for `call`, sending nothing. `Ok(None)` is no grant: an anonymous read.
/// Otherwise the envelope the call may present.
///
/// Refused as `Denied`: no grant for a write, an envelope that is not one, one for another
/// registry, one of a phase the verb does not accept, and one that leaves any coordinate of the
/// call uncovered (every one is checked, not the first). Refused as `Expired`: a grant whose
/// stated expiry is not after now plus `lasts_ms`. A grant with a wrong repository is `Denied`
/// whether or not it is also expired.
pub fn authorise(grant: &Grant, call: &Use<'_>) -> Result<Option<Envelope>, SeamError> {
    let _ = (grant, call);
    Err(denied())
}
