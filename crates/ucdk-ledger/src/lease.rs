// SPDX-License-Identifier: Apache-2.0
//! The lease: exclusive use of a ledger for one holder at a time, with a fencing token.
//!
//! A holder takes the lease with [`Ledger::acquire`](crate::Ledger::acquire) and gets a
//! [`Lease`]: its identity, a [`Fence`] and the time it expires. Every write it makes names the
//! fence ([`Ledger::write_fenced`](crate::Ledger::write_fenced)), and the ledger refuses a write
//! whose fence is not the current one. The fence only goes up: each grant is one more than the
//! last, whether the earlier lease was released or lapsed, so a holder whose lease lapsed and
//! was taken over cannot write again, however long it was stalled and whether or not it knows.
//!
//! # Time and skew
//!
//! The ledger judges expiry, by its own [`Clock`]; a holder's clock never decides anything. A
//! lease granted at `t` for `ttl` lapses at `t + ttl`: from then the holder's writes and renewals
//! are refused. A new holder is granted the lease only once the ledger's clock reads
//! `lapse + MAX_CLOCK_SKEW`. The gap bounds the disagreement between the clocks of two handles
//! on one document (two machines on a shared file system, say): a handle whose clock runs up to
//! `MAX_CLOCK_SKEW` ahead of the holder's still cannot grant before the holder's clock has passed
//! the lapse, and from then the holder is refused. Skew beyond the bound is not covered, and
//! neither is a clock that steps back; the fence still refuses a holder that was overtaken, and
//! the bound only matters for a lease that has lapsed but not been taken.
//!
//! Fencing protects the ledger, not what a holder does outside it: a stalled holder's call that
//! is already in flight is not recalled. The write-ahead intent is what makes that call
//! resolvable by the next holder.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ucdk_core::millis;
// The clocks live in `ucdk-core`, so the pure crates can move one by hand; they are re-exported
// here at the paths they always had.
pub use ucdk_core::{Clock, ManualClock};

/// How far two clocks reading one lease may disagree before the lease's guarantee is lost.
pub const MAX_CLOCK_SKEW: Duration = Duration::from_secs(5);

/// The wall clock. This is the one reader of `SystemTime` in non-test code.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        millis(since)
    }
}

/// The fencing token of a lease. Each grant has a higher one than every grant before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Fence(pub(crate) u64);

impl Fence {
    /// A fence no lease ever carries (the first grant is 1): a write under it is always
    /// refused. It is what a holder has before it has acquired.
    pub const NONE: Self = Self(0);

    /// The fence numbered `value`, for one carried from elsewhere (a log line, a message). The
    /// ledger accepts only the one it granted last.
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// The number, which only goes up.
    pub fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Fence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "f{}", self.0)
    }
}

/// A granted lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    pub holder: String,
    pub fence: Fence,
    /// When the lease lapses, in milliseconds on the ledger's clock.
    pub expires_at: u64,
}

/// The lease as a document records it: who holds it, under which fence, until when. It says
/// nothing of whether the lease is still good; [`Holding::state`] judges that against a reading
/// of the ledger's clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holding {
    pub holder: String,
    pub fence: Fence,
    /// When the lease lapses, in milliseconds on the ledger's clock.
    pub expires_at: u64,
}

/// Where a recorded lease stands at one instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaseState {
    /// Before `expires_at`: the holder may write, and no other holder is granted the lease.
    Live,
    /// From `expires_at` until `expires_at` plus [`MAX_CLOCK_SKEW`]: the holder may no longer
    /// write, and another holder is still refused.
    Lapsing,
    /// After that: another holder is granted the lease.
    Lapsed,
}

impl Holding {
    /// The state of the lease at `at`, a reading of the ledger's clock. This is the line the
    /// ledger draws when it grants the lease and when it admits an unfenced write.
    pub fn state(&self, at: u64) -> LeaseState {
        if at < self.expires_at {
            LeaseState::Live
        } else if at < self.expires_at.saturating_add(millis(MAX_CLOCK_SKEW)) {
            LeaseState::Lapsing
        } else {
            LeaseState::Lapsed
        }
    }
}

/// Why a lease was not granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseError {
    /// Another holder has the lease, or its lapse is not yet past the skew bound.
    Held { holder: String, remaining: Duration },
    /// The ledger could not be reached or written: nothing was granted.
    Unavailable(String),
    /// The ledger was opened to read and does not write: nothing was granted.
    ReadOnly,
    /// A conditional grant ([`Ledger::acquire_if`](crate::Ledger::acquire_if)) found that the
    /// document, or the highest fence ever granted, is no longer what the caller read: nothing
    /// was granted.
    Changed,
}

impl fmt::Display for LeaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Held { holder, remaining } => write!(
                f,
                "the ledger is leased to {holder} for at least {} more second(s)",
                remaining.as_secs().max(1)
            ),
            Self::Unavailable(why) => write!(f, "the lease is unavailable: {why}"),
            Self::ReadOnly => f.write_str("the ledger was opened read-only: no lease is granted"),
            Self::Changed => f.write_str(
                "the ledger changed since it was read, or a lease was granted since: \
                 no lease is granted",
            ),
        }
    }
}

impl std::error::Error for LeaseError {}
