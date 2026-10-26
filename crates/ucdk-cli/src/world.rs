// SPDX-License-Identifier: Apache-2.0
//! The world a command runs in: the clock a lease is judged by and the two output buffers.

use std::sync::Arc;

use ucdk_ledger::{Clock, SystemClock};

/// Everything a command reads besides the files it is given, and where it writes.
///
/// `run` appends what the command prints to `out` and `err`; the binary writes them to the
/// process streams when the command is done. The clock is the one a lease is judged by: the
/// wall clock, or, in a test, one the test moves.
#[derive(Debug)]
pub struct World {
    /// The clock a lease is reported live, lapsing or lapsed by.
    pub clock: Arc<dyn Clock>,
    /// What the command printed to standard output.
    pub out: String,
    /// What the command printed to standard error.
    pub err: String,
}

impl World {
    /// The wall clock and nothing printed.
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    /// [`World::new`], with leases judged by `clock`: how a test says when a lease lapses
    /// without waiting.
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            out: String::new(),
            err: String::new(),
        }
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}
