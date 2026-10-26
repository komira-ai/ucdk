// SPDX-License-Identifier: Apache-2.0
//! The clock a ledger reads, and the one a test moves by hand. The one reader of the wall clock
//! is `SystemClock`, in `ucdk-ledger`, so this crate stays free of it.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A source of the time, in milliseconds since the Unix epoch. A ledger holds one, so a test
/// can move time by hand.
pub trait Clock: Send + Sync + fmt::Debug {
    fn now_millis(&self) -> u64;
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
pub fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
