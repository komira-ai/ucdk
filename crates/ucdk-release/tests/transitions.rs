// SPDX-License-Identifier: Apache-2.0
//! The state-transition table of a release record.

use ucdk_release::transitions::{Event, Kind, Refused, Stand, transition};

use Event::{Adopt, BeginReap, Confirm, CreateConfirmed, Drop, WriteIntent};
use Kind::{Action, Grant, Publication, Run, Seal, Stage, Validation};
use Stand::{Absent, Confirmed, Intent, Reaping};

/// The kinds that are the work of a run and may be announced for removal once confirmed; every
/// other kind is audit data once confirmed. Of the two, only a validation is written again.
const RERUNNABLE: [Kind; 2] = [Action, Validation];
const SETTLED: [Kind; 5] = [Run, Stage, Publication, Grant, Seal];

#[test]
fn total_over_every_state_and_event() {
    // One row per state, one column per event, in the order of `Event::ALL`: the state that
    // follows, or `None` for a refusal. Every pair has an entry, so a state or an event added
    // without a decision here fails this test, and the table is the document. Two things depend
    // on the kind: a confirmed validation is written as an intent again (a read-back is never
    // skipped), a confirmed action or validation may begin to be reaped, and a record that does
    // not exist is created confirmed by one event only, `Adopt` for a publication and
    // `CreateConfirmed` for a seal.
    let table = |kind: Kind| -> [(Stand, [Option<Stand>; 6]); 4] {
        let reopen = (kind == Validation).then_some(Intent);
        let reap = RERUNNABLE.contains(&kind).then_some(Reaping);
        let adopt = (kind == Publication).then_some(Confirmed);
        let create = (kind == Seal).then_some(Confirmed);
        [
            //   intent        confirm  adopt  create  begin reap  drop
            (Absent, [Some(Intent), None, adopt, create, None, None]),
            (
                Intent,
                [
                    Some(Intent),
                    Some(Confirmed),
                    None,
                    None,
                    Some(Reaping),
                    Some(Absent),
                ],
            ),
            (Confirmed, [reopen, None, None, None, reap, None]),
            (
                Reaping,
                [None, None, None, None, Some(Reaping), Some(Absent)],
            ),
        ]
    };
    assert_eq!(
        Event::ALL,
        [
            WriteIntent,
            Confirm,
            Adopt,
            CreateConfirmed,
            BeginReap,
            Drop
        ]
    );
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
fn a_seal_is_created_confirmed_in_one_write() {
    // A seal is one fenced write under the seal lease, with `Confirmed` state: no intent comes
    // first, and the record that is written is settled.
    assert_eq!(transition(Seal, Absent, CreateConfirmed), Ok(Confirmed));
    // Once it is there it is audit data: it is not created again, not opened by an intent, not
    // reaped, and not dropped, so no sequence takes a seal back to absent.
    for event in Event::ALL {
        assert!(
            transition(Seal, Confirmed, event).is_err(),
            "a confirmed seal + {event:?}"
        );
    }
    // A seal that is found is not adopted: adoption is of what a registry holds.
    assert_eq!(
        transition(Seal, Absent, Adopt),
        Err(Refused {
            from: Absent,
            event: Adopt
        })
    );
}

#[test]
fn only_one_write_kinds_may_be_created_confirmed() {
    // The kinds the design writes in one fenced write are the seal alone. Every other kind
    // goes through an intent first, or is found: a run, stage, action, validation and grant are
    // not created confirmed, and a publication reaches `Confirmed` from nothing only by being
    // adopted.
    for kind in Kind::ALL {
        let created = transition(kind, Absent, CreateConfirmed);
        assert_eq!(created.is_ok(), kind == Seal, "{kind:?}: {created:?}");
        assert_eq!(kind.is_created_confirmed(), kind == Seal, "{kind:?}");
        // The only ways a missing record becomes confirmed.
        let adopted = transition(kind, Absent, Adopt);
        assert_eq!(
            adopted.is_ok(),
            kind == Publication,
            "{kind:?}: {adopted:?}"
        );
        for event in [WriteIntent, Confirm, BeginReap, Drop] {
            assert_ne!(
                transition(kind, Absent, event),
                Ok(Confirmed),
                "{kind:?} {event:?}"
            );
        }
        // Neither event applies to a record that exists.
        for from in [Intent, Confirmed, Reaping] {
            for event in [Adopt, CreateConfirmed] {
                assert_eq!(
                    transition(kind, from, event),
                    Err(Refused { from, event }),
                    "{kind:?}: {from:?} + {event:?}"
                );
            }
        }
    }
}

#[test]
fn a_confirmed_stage_is_not_reopened_by_a_rerun_of_the_same_run() {
    // A rerun under the same run id finds a stage record whose `confirmed` is written, and
    // leaves it as it is: the table refuses a new intent (the new attempt), a second
    // confirmation, a removal and a drop, so no rerun can clear `confirmed` or take the record
    // back to a state where the stage runs again. The rerun is a `NOOP` that writes no stage
    // record; a new attempt of that work needs a new run id. A stage that is not confirmed is
    // written as an intent again, which is how a resume records its new attempt.
    for event in Event::ALL {
        assert_eq!(
            transition(Stage, Confirmed, event),
            Err(Refused {
                from: Confirmed,
                event
            }),
            "a confirmed stage + {event:?}"
        );
    }
    assert_eq!(transition(Stage, Intent, WriteIntent), Ok(Intent));
    // Nor does a rerun reopen what the stage published or sealed, or the run it belongs to.
    for kind in [Run, Publication, Grant, Seal] {
        assert!(
            transition(kind, Confirmed, WriteIntent).is_err(),
            "{kind:?}"
        );
    }
}

#[test]
fn a_confirmed_action_is_not_reopened() {
    // Under one run id a confirmed action with the same inputs is skipped (`NOOP`), and one with
    // other inputs is refused (`INPUTS_CHANGED`), including when it is selected alone with
    // `--only action:<name>`; so no engine path writes an intent over it, and the table
    // refuses one. The work is done again under a new run id, whose ledger has no such record.
    assert_eq!(
        transition(Action, Confirmed, WriteIntent),
        Err(Refused {
            from: Confirmed,
            event: WriteIntent
        })
    );
    // An action that is not confirmed is written again: a failed test is run again.
    assert_eq!(transition(Action, Intent, WriteIntent), Ok(Intent));
    assert_eq!(transition(Action, Absent, WriteIntent), Ok(Intent));
}

#[test]
fn a_confirmed_validation_may_be_rewritten() {
    // A read-back is never skipped, whatever its record says: the registry is the truth and may
    // have changed. Its record is written as an intent again under each run, and confirmed
    // again when it passes. A failed read-back therefore leaves the validation as an intent.
    assert_eq!(transition(Validation, Confirmed, WriteIntent), Ok(Intent));
    assert_eq!(transition(Validation, Intent, Confirm), Ok(Confirmed));
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
    // The one kind written again once confirmed is a validation; an action is not.
    for kind in Kind::ALL {
        assert_eq!(
            transition(kind, Confirmed, WriteIntent).is_ok(),
            kind == Validation,
            "{kind:?}"
        );
        assert_eq!(kind.is_rewritten_once_confirmed(), kind == Validation);
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
