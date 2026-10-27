// SPDX-License-Identifier: Apache-2.0
//! The only time seams: counted polls and a heartbeat. Neither reads a clock.

/// A lease was fenced or lapsed: stop what you started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stop;

/// Called on each progress event of a long call. The engine renews the leases it holds when
/// they are due; `Err(Stop)` means one is lost and the caller must stop and kill what it started.
pub trait Heartbeat {
    fn beat(&self) -> Result<(), Stop>;
}

/// The wait between polls and retries: `ticks` ticks, never a sleep of the seam's own. A test
/// counts the ticks, so no test depends on how long anything took.
pub trait Pause {
    fn pause(&mut self, ticks: u32);
}
