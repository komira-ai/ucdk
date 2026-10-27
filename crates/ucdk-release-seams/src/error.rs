// SPDX-License-Identifier: Apache-2.0
//! The error every seam returns. It says what the caller may assume about the effect: a bare
//! failure that cannot tell whether the bytes left is never `Permanent`.

use std::error::Error;
use std::fmt;

use ucdk_release::codec::GrantFault;
use ucdk_release_proto::v1::Reason;

/// How long a refusal asks the caller to wait, in ticks of [`crate::Pause`].
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

impl SeamError {
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
