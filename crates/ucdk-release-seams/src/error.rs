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

impl Backoff {
    /// The backoff for a wait of `ms` milliseconds, rounded up to whole ticks of
    /// [`crate::PAUSE_TICK_MS`] so that a registry never asks for less than it was told, and
    /// saturating at `u32::MAX` ticks. A registry that is told to wait in seconds or
    /// milliseconds states its backoff with this and keeps no conversion of its own.
    pub fn after_millis(ms: u64) -> Self {
        let tick = u64::from(crate::PAUSE_TICK_MS);
        Self {
            ticks: u32::try_from(ms.div_ceil(tick)).unwrap_or(u32::MAX),
        }
    }

    /// The backoff for a wait of `seconds` seconds, as [`Backoff::after_millis`].
    pub fn after_seconds(seconds: u32) -> Self {
        Self::after_millis(u64::from(seconds) * 1000)
    }
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

/// Where in an action a failed `observe` ran. It decides what the failure says about the action,
/// because an observe that fails after an effect may have landed cannot say that nothing happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveWhen {
    /// The preflight observe, before any registry write: nothing of the action has landed.
    Preflight,
    /// An observe after an effect that may have landed (the observe that follows a `Duplicate`,
    /// the observe that follows a commit answer, the read-back poll). Its failure concerns the
    /// effect, which is unknown.
    AfterEffect,
}

impl SeamError {
    /// What a failed `observe` makes of the action, given when it ran.
    ///
    /// `Preflight`, where nothing was written: `Refused(r)` says nothing happened and is the
    /// action's refusal with that same reason `r`; `Permanent` says nothing happened and the same
    /// call fails again (a coordinate the channel cannot hold, a set that does not contain it),
    /// so it is `REFUSED` with `CHANNEL`; `GrantRefused(Denied)` is `REFUSED` with `CREDENTIAL`.
    /// Every other failure leaves the registry unread and the action `INDETERMINATE`.
    ///
    /// `AfterEffect`: every failure is `INDETERMINATE`, whatever it promises. A refusal of the
    /// observe says nothing of the effect before it, and that effect is unknown.
    pub fn observe_failure(&self, when: ObserveWhen) -> ObserveFailure {
        if when == ObserveWhen::AfterEffect {
            return ObserveFailure::Indeterminate;
        }
        match self {
            SeamError::Refused(reason) => ObserveFailure::Refused(*reason),
            SeamError::Permanent(_) => ObserveFailure::Refused(Reason::Channel),
            SeamError::GrantRefused(GrantFault::Denied) => {
                ObserveFailure::Refused(Reason::Credential)
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
