// SPDX-License-Identifier: Apache-2.0
//! The error every seam returns. It says what the caller may assume about the effect: a bare
//! failure that cannot tell whether the bytes left is never `Permanent`.

use std::error::Error;
use std::fmt;

use ucdk_release::codec::GrantFault;
use ucdk_release_proto::v1::Reason;

/// How long a refusal asks the caller to wait, in ticks of [`crate::Pause`]: `ticks` of
/// [`crate::PAUSE_TICK_MS`] milliseconds each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub ticks: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeamError {
    /// Nothing happened; do not retry as is.
    Refused(Reason),
    /// It will fail again; nothing happened.
    Permanent(String),
    /// It failed and may have landed: observe, then retry only if absent.
    Transient(String),
    /// Refused before it ran; retry after the stated number of pause ticks.
    RateLimited(Backoff),
    /// Something may be underway; observe.
    InFlight(String),
    /// The answer was lost and the effect may have landed; observe.
    Unknown(String),
    /// The grant presented was refused before the call ran.
    GrantRefused(GrantFault),
}

/// What a failed `observe` makes of the action that called it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveFailure {
    /// The action is `REFUSED` with this reason and nothing is retried.
    Refused(Reason),
    /// The action is `INDETERMINATE`: the registry could not be read, and a later run may.
    Indeterminate,
}

impl SeamError {
    /// What a failed `observe` is. An observe that answers `Refused` or `Permanent` says that
    /// nothing happened and that the same call fails again (a coordinate the channel cannot
    /// hold, a set that does not contain it), so it is not a retry class: the action is
    /// `REFUSED` with `CHANNEL`. Every other failure leaves the registry unread, and the action
    /// is `INDETERMINATE`.
    pub fn observe_failure(&self) -> ObserveFailure {
        match self {
            SeamError::Refused(_) | SeamError::Permanent(_) => {
                ObserveFailure::Refused(Reason::Channel)
            }
            SeamError::Transient(_)
            | SeamError::RateLimited(_)
            | SeamError::InFlight(_)
            | SeamError::Unknown(_)
            | SeamError::GrantRefused(_) => ObserveFailure::Indeterminate,
        }
    }

    /// Whether the variant promises that no external effect landed.
    pub fn promises_no_effect(&self) -> bool {
        matches!(
            self,
            SeamError::Refused(_)
                | SeamError::Permanent(_)
                | SeamError::RateLimited(_)
                | SeamError::GrantRefused(_)
        )
    }
}

impl fmt::Display for SeamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SeamError::Refused(reason) => write!(f, "refused ({reason:?}); nothing happened"),
            SeamError::Permanent(why) => write!(f, "failed for good, nothing happened: {why}"),
            SeamError::Transient(why) => write!(f, "failed, and it may have landed: {why}"),
            SeamError::RateLimited(backoff) => {
                write!(f, "rate limited; retry after {} ticks", backoff.ticks)
            }
            SeamError::InFlight(why) => write!(f, "something may be underway: {why}"),
            SeamError::Unknown(why) => write!(f, "the answer was lost: {why}"),
            SeamError::GrantRefused(fault) => {
                write!(f, "the grant was refused ({fault:?}); nothing happened")
            }
        }
    }
}

impl Error for SeamError {}
