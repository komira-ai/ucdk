// SPDX-License-Identifier: Apache-2.0
//! The state-transition table of a release record.

use ucdk_release::transitions::{Event, Refused, Stand, transition};

use Event::{Adopt, BeginReap, Confirm, Drop, WriteIntent};
use Stand::{Absent, Confirmed, Intent, Reaping};

#[test]
fn total_over_every_state_and_event() {
    // One row per state, one column per event, in the order of `Event::ALL`: the state that
    // follows, or `None` for a refusal. Every pair has an entry, so a state or an event added
    // without a decision here fails this test, and the table is the document.
    let table: [(Stand, [Option<Stand>; 5]); 4] = [
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
        (Confirmed, [Some(Intent), None, None, Some(Reaping), None]),
        (Reaping, [None, None, None, Some(Reaping), Some(Absent)]),
    ];
    assert_eq!(Event::ALL, [WriteIntent, Confirm, Adopt, BeginReap, Drop]);
    assert_eq!(table.map(|(s, _)| s), Stand::ALL);
    for (from, row) in table {
        for (event, wanted) in Event::ALL.into_iter().zip(row) {
            let got = transition(from, event);
            match wanted {
                Some(next) => assert_eq!(got, Ok(next), "{from:?} + {event:?}"),
                None => assert_eq!(got, Err(Refused { from, event }), "{from:?} + {event:?}"),
            }
        }
    }
}

#[test]
fn a_confirmed_record_is_never_dropped_and_a_record_in_removal_is_never_confirmed() {
    for event in Event::ALL {
        if let Ok(next) = transition(Confirmed, event) {
            assert_ne!(next, Absent, "{event:?}");
        }
        if let Ok(next) = transition(Reaping, event) {
            assert_ne!(next, Confirmed, "{event:?}");
            assert_ne!(next, Intent, "{event:?}");
        }
    }
}

#[test]
fn a_refusal_names_the_state_and_the_event() {
    let refusal = transition(Reaping, Confirm).expect_err("refused");
    let text = refusal.to_string();
    assert!(
        text.contains("Reaping") && text.contains("Confirm"),
        "{text}"
    );
}
