// SPDX-License-Identifier: Apache-2.0
//! The state-transition table of a release record.

use ucdk_release::transitions::{Event, Kind, Refused, Stand, transition};

use Event::{Adopt, BeginReap, Confirm, Drop, WriteIntent};
use Kind::{Action, Grant, Publication, Run, Seal, Stage, Validation};
use Stand::{Absent, Confirmed, Intent, Reaping};

/// The kinds whose work a run may do again; every other kind is audit data once confirmed.
const RERUNNABLE: [Kind; 2] = [Action, Validation];
const SETTLED: [Kind; 5] = [Run, Stage, Publication, Grant, Seal];

#[test]
fn total_over_every_state_and_event() {
    // One row per state, one column per event, in the order of `Event::ALL`: the state that
    // follows, or `None` for a refusal. Every pair has an entry, so a state or an event added
    // without a decision here fails this test, and the table is the document. The confirmed
    // row is the one that depends on the kind: a kind that may be run again may be written as
    // an intent again or begin to be reaped.
    let table = |kind: Kind| -> [(Stand, [Option<Stand>; 5]); 4] {
        let reopen = RERUNNABLE.contains(&kind).then_some(Intent);
        let reap = RERUNNABLE.contains(&kind).then_some(Reaping);
        [
            //            intent         confirm          adopt            begin reap     drop
            (Absent, [Some(Intent), None, Some(Confirmed), None, None]),
            (
                Intent,
                [
                    Some(Intent),
                    Some(Confirmed),
                    None,
                    Some(Reaping),
                    Some(Absent),
                ],
            ),
            (Confirmed, [reopen, None, None, reap, None]),
            (Reaping, [None, None, None, Some(Reaping), Some(Absent)]),
        ]
    };
    assert_eq!(Event::ALL, [WriteIntent, Confirm, Adopt, BeginReap, Drop]);
    assert_eq!(
        Kind::ALL,
        [Run, Stage, Action, Publication, Validation, Grant, Seal]
    );
    for kind in Kind::ALL {
        assert_eq!(table(kind).map(|(s, _)| s), Stand::ALL);
        for (from, row) in table(kind) {
            for (event, wanted) in Event::ALL.into_iter().zip(row) {
                let got = transition(kind, from, event);
                match wanted {
                    Some(next) => assert_eq!(got, Ok(next), "{kind:?}: {from:?} + {event:?}"),
                    None => assert_eq!(
                        got,
                        Err(Refused { from, event }),
                        "{kind:?}: {from:?} + {event:?}"
                    ),
                }
            }
        }
    }
}

#[test]
fn a_confirmed_record_is_never_reopened() {
    // A publication, seal, run, stage or grant that is confirmed is audit data.
    for kind in SETTLED {
        assert_eq!(
            transition(kind, Confirmed, WriteIntent),
            Err(Refused {
                from: Confirmed,
                event: WriteIntent
            }),
            "{kind:?}"
        );
    }
    // The kinds whose work a run may do again are written as an intent again.
    for kind in RERUNNABLE {
        assert_eq!(
            transition(kind, Confirmed, WriteIntent),
            Ok(Intent),
            "{kind:?}"
        );
    }
    // The two lists cover every kind once.
    assert_eq!(SETTLED.len() + RERUNNABLE.len(), Kind::ALL.len());
    for kind in Kind::ALL {
        assert_eq!(
            SETTLED.contains(&kind),
            kind.is_settled_once_confirmed(),
            "{kind:?}"
        );
        assert_ne!(
            SETTLED.contains(&kind),
            RERUNNABLE.contains(&kind),
            "{kind:?}"
        );
    }
    // An intent that never landed is written again by any kind.
    for kind in Kind::ALL {
        assert_eq!(
            transition(kind, Intent, WriteIntent),
            Ok(Intent),
            "{kind:?}"
        );
    }
}

#[test]
fn a_confirmed_record_is_never_dropped_and_a_record_in_removal_is_never_confirmed() {
    for kind in Kind::ALL {
        for event in Event::ALL {
            if let Ok(next) = transition(kind, Confirmed, event) {
                assert_ne!(next, Absent, "{kind:?} {event:?}");
            }
            if let Ok(next) = transition(kind, Reaping, event) {
                assert_ne!(next, Confirmed, "{kind:?} {event:?}");
                assert_ne!(next, Intent, "{kind:?} {event:?}");
            }
        }
    }
}

#[test]
fn no_sequence_of_events_takes_a_settled_confirmed_record_to_absent() {
    // Search every sequence of up to four events from `Confirmed`, so a two-step path through
    // `Reaping` is seen as well as a single event.
    for kind in SETTLED {
        let mut frontier = vec![Confirmed];
        for _ in 0..4 {
            let mut next_frontier = Vec::new();
            for from in frontier {
                for event in Event::ALL {
                    if let Ok(next) = transition(kind, from, event) {
                        assert_ne!(next, Absent, "{kind:?}: {from:?} + {event:?}");
                        next_frontier.push(next);
                    }
                }
            }
            frontier = next_frontier;
        }
        // Nothing leaves `Confirmed` at all.
        for event in Event::ALL {
            assert!(
                transition(kind, Confirmed, event).is_err(),
                "{kind:?} {event:?}"
            );
        }
    }
    // A re-runnable record is the one that can be announced for removal after it was confirmed.
    assert_eq!(transition(Action, Confirmed, BeginReap), Ok(Reaping));
    assert_eq!(transition(Action, Reaping, Drop), Ok(Absent));
}

#[test]
fn a_refusal_names_the_state_and_the_event() {
    let refusal = transition(Run, Reaping, Confirm).expect_err("refused");
    let text = refusal.to_string();
    assert!(
        text.contains("Reaping") && text.contains("Confirm"),
        "{text}"
    );
}

#[test]
fn a_kind_is_known_by_its_tag() {
    for kind in Kind::ALL {
        assert_eq!(Kind::from_tag(kind.tag()), Some(kind));
    }
    assert_eq!(Kind::from_tag("step"), None);
    assert_eq!(Kind::from_tag("created"), None);
    assert_eq!(Kind::from_tag(""), None);
}
