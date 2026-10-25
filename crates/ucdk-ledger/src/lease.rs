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
//! Fencing protects the ledger, not the cloud: a stalled holder's provider call that is already
//! in flight is not recalled. The write-ahead intent and the stamp are what make that call
//! resolvable by the next holder.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How far two clocks reading one lease may disagree before the lease's guarantee is lost.
pub const MAX_CLOCK_SKEW: Duration = Duration::from_secs(5);

/// A source of the time, in milliseconds since the Unix epoch. A ledger holds one, so a test
/// can move time by hand.
pub trait Clock: Send + Sync + fmt::Debug {
    fn now_millis(&self) -> u64;
}

/// The wall clock.
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

/// A clock that moves only when told to. A clone shares the time.
#[derive(Debug, Clone, Default)]
pub struct ManualClock {
    now: Arc<AtomicU64>,
}

impl ManualClock {
    /// A clock reading `start` milliseconds.
    pub fn at(start: u64) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(start)),
        }
    }

    /// Moves the clock forward.
    pub fn advance(&self, by: Duration) {
        self.now.fetch_add(millis(by), Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_millis(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

/// A duration in milliseconds, saturating.
pub(crate) fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
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

/// Why a lease was not granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseError {
    /// Another holder has the lease, or its lapse is not yet past the skew bound.
    Held { holder: String, remaining: Duration },
    /// The ledger could not be reached or written: nothing was granted.
    Unavailable(String),
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
        }
    }
}

impl std::error::Error for LeaseError {}
