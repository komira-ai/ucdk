// SPDX-License-Identifier: Apache-2.0
//! The counted pause and the counted heartbeat.

use ucdk_core::ManualClock;
use ucdk_release_fake::{FakeHeartbeat, FakePause};
use ucdk_release_seams::{Heartbeat, Pause, Stop};

#[test]
fn a_pause_is_counted_and_waits_for_nothing() {
    let mut pause = FakePause::new();
    pause.pause(1);
    pause.pause(4);
    assert_eq!(pause.ticks(), [1, 4]);
    assert_eq!(pause.total(), 5);
}

#[test]
fn a_heartbeat_records_the_clock_and_stops_from_the_beat_it_was_told() {
    let clock = ManualClock::at(10);
    let beat = FakeHeartbeat::new(&clock).stop_at(2);
    assert_eq!(beat.beat(), Ok(()));
    clock.advance(std::time::Duration::from_millis(5));
    assert_eq!(beat.beat(), Err(Stop));
    assert_eq!(beat.beat(), Err(Stop));
    assert_eq!(beat.readings(), [10, 15, 15]);
    assert_eq!(beat.beats(), 3);
    assert_eq!(FakeHeartbeat::new(&clock).beat(), Ok(()));
}
