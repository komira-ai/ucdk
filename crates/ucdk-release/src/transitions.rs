// SPDX-License-Identifier: Apache-2.0
//! The states a release record passes through, and the one table of which events move a record
//! from which state to which. The table is total: every kind, state and event has an answer,
//! either the state that follows or a refusal. A writer asks the table before it writes.

use std::fmt;

/// Where a record stands. A record that does not exist is a state too, [`Stand::Absent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stand {
    Absent,
    /// Written before an effect: the effect may or may not have landed.
    Intent,
    /// The effect landed and was read back, or the work passed.
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

/// The kind of a record: the tag it is written under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Run,
    Stage,
    Action,
    Publication,
    Validation,
    Grant,
    Seal,
}

impl Kind {
    pub const ALL: [Kind; 7] = [
        Kind::Run,
        Kind::Stage,
        Kind::Action,
        Kind::Publication,
        Kind::Validation,
        Kind::Grant,
        Kind::Seal,
    ];

    /// The tag a record of this kind is written under.
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Run => "run",
            Kind::Stage => "stage",
            Kind::Action => "action",
            Kind::Publication => "publication",
            Kind::Validation => "validation",
            Kind::Grant => "grant",
            Kind::Seal => "seal",
        }
    }

    /// The kind written under `tag`, if there is one.
    pub fn from_tag(tag: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.tag() == tag)
    }

    /// Whether a confirmed record of this kind is audit data or a fact that later attempts
    /// build on, so it is never written as an intent again. An action or a validation is the
    /// work of a run and may be run again.
    pub fn is_settled_once_confirmed(self) -> bool {
        !matches!(self, Kind::Action | Kind::Validation)
    }
}

/// The state a record of `kind` is in after `event`, or the refusal.
///
/// A confirmed record is never dropped: it is audit data. A confirmed run, stage, publication,
/// grant or seal is not re-opened by a new intent and does not begin to be reaped either, so no
/// path of events takes it to absent; a confirmed action or validation may be written as an
/// intent again, because a run may do the work again, or announced for removal. A record being
/// reaped is dropped only when its removal finished, and is never confirmed again.
pub fn transition(kind: Kind, from: Stand, event: Event) -> Result<Stand, Refused> {
    let next = match (from, event) {
        (Stand::Confirmed, Event::WriteIntent | Event::BeginReap)
            if kind.is_settled_once_confirmed() =>
        {
            // Announcing removal of a settled confirmed record would reach `Absent` in two
            // events.
            return Err(Refused { from, event });
        }
        (Stand::Absent | Stand::Intent | Stand::Confirmed, Event::WriteIntent) => Stand::Intent,
        (Stand::Intent, Event::Confirm) => Stand::Confirmed,
        (Stand::Absent, Event::Adopt) => Stand::Confirmed,
        (Stand::Intent | Stand::Confirmed | Stand::Reaping, Event::BeginReap) => Stand::Reaping,
        (Stand::Intent | Stand::Reaping, Event::Drop) => Stand::Absent,
        _ => return Err(Refused { from, event }),
    };
    Ok(next)
}
