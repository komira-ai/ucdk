// SPDX-License-Identifier: Apache-2.0
//! Counted polls and counted beats: no test waits for anything.

use std::sync::{Mutex, PoisonError};

use ucdk_core::{Clock, ManualClock};
use ucdk_release_seams::{Heartbeat, Pause, Stop};

/// A [`Pause`] that records the ticks it was asked to wait and waits for none.
#[derive(Debug, Default)]
pub struct FakePause {
    ticks: Vec<u32>,
}

impl FakePause {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every pause, in order.
    pub fn ticks(&self) -> &[u32] {
        &self.ticks
    }

    pub fn total(&self) -> u64 {
        self.ticks.iter().map(|t| u64::from(*t)).sum()
    }
}

impl Pause for FakePause {
    fn pause(&mut self, ticks: u32) {
        self.ticks.push(ticks);
    }
}

/// A [`Heartbeat`] that records the reading of the shared clock at each beat, and answers
/// `Stop` from the beat it was told to.
#[derive(Debug)]
pub struct FakeHeartbeat {
    clock: ManualClock,
    seen: Mutex<Vec<u64>>,
    stop_at: Option<usize>,
}

impl FakeHeartbeat {
    pub fn new(clock: &ManualClock) -> Self {
        Self {
            clock: clock.clone(),
            seen: Mutex::new(Vec::new()),
            stop_at: None,
        }
    }

    /// Answers `Stop` from the `n`th beat on (1 is the first).
    #[must_use]
    pub fn stop_at(mut self, n: usize) -> Self {
        self.stop_at = Some(n);
        self
    }

    /// The clock's reading at each beat, in order.
    pub fn readings(&self) -> Vec<u64> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub fn beats(&self) -> usize {
        self.readings().len()
    }
}

impl Heartbeat for FakeHeartbeat {
    fn beat(&self) -> Result<(), Stop> {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        seen.push(self.clock.now_millis());
        if self.stop_at.is_some_and(|n| seen.len() >= n) {
            Err(Stop)
        } else {
            Ok(())
        }
    }
}
