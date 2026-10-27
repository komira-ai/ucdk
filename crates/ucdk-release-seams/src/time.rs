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

/// The duration of one [`Pause`] tick, in milliseconds. Every count of ticks in the seams (the
/// wait between polls, [`crate::Backoff::ticks`]) is in this unit, so a registry converts a
/// wait it is told in seconds to ticks with this constant and no setting of its own.
pub const PAUSE_TICK_MS: u32 = 1000;

/// The wait between polls and retries: `ticks` ticks of [`PAUSE_TICK_MS`] milliseconds each,
/// never a sleep of the seam's own. A real implementation sleeps `ticks * PAUSE_TICK_MS`
/// milliseconds; a test counts the ticks, so no test depends on how long anything took.
pub trait Pause {
    fn pause(&mut self, ticks: u32);
}
