// SPDX-License-Identifier: Apache-2.0
//! The states a release record passes through, and the one table of which events move a record
//! from which state to which. The table is total: every state and event has an answer, either
//! the state that follows or a refusal. A writer asks the table before it writes.

use std::fmt;

/// Where a record stands. A record that does not exist is a state too, [`Stand::Absent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stand {
    Absent,
    /// Written before an effect: the effect may or may not have landed.
    Intent,
    /// The effect landed and was read back.
    Confirmed,
    /// Written before the thing is removed: it may or may not be gone.
    Reaping,
}

/// What happens to a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Intent is written, or written again after a restart or a new attempt.
    WriteIntent,
    /// The effect is read back and the record confirmed.
    Confirm,
    /// What was found there is taken over without an intent of ours: written confirmed.
    Adopt,
    /// Removal is announced before the thing is removed.
    BeginReap,
    /// The record is dropped: an intent whose effect did not land, or a removal that finished.
    Drop,
}

impl Stand {
    pub const ALL: [Stand; 4] = [
        Stand::Absent,
        Stand::Intent,
        Stand::Confirmed,
        Stand::Reaping,
    ];
}

impl Event {
    pub const ALL: [Event; 5] = [
        Event::WriteIntent,
        Event::Confirm,
        Event::Adopt,
        Event::BeginReap,
        Event::Drop,
    ];
}

/// An event that the state it met does not allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused {
    pub from: Stand,
    pub event: Event,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a record that is {:?} cannot take the event {:?}",
            self.from, self.event
        )
    }
}

/// The state a record is in after `event`, or the refusal.
///
/// A confirmed record is never dropped: it is audit data. A record being reaped is dropped only
/// when its removal finished, and is never confirmed again.
pub fn transition(from: Stand, event: Event) -> Result<Stand, Refused> {
    let next = match (from, event) {
        (Stand::Absent | Stand::Intent | Stand::Confirmed, Event::WriteIntent) => Stand::Intent,
        (Stand::Intent, Event::Confirm) => Stand::Confirmed,
        (Stand::Absent, Event::Adopt) => Stand::Confirmed,
        (Stand::Intent | Stand::Confirmed | Stand::Reaping, Event::BeginReap) => Stand::Reaping,
        (Stand::Intent | Stand::Reaping, Event::Drop) => Stand::Absent,
        _ => return Err(Refused { from, event }),
    };
    Ok(next)
}
