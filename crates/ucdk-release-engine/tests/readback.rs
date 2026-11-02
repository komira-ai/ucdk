// SPDX-License-Identifier: Apache-2.0
//! Polling of a read-back, one coordinate at a time: what a read that fails, a coordinate that
//! is not served yet and a bound that runs out each cost in reads and pauses, and the answers
//! that end the polling at once. Answers are scripted per call on the rig's publisher.

mod common;

use common::{Fetch, Rig, coordinate, readback_row};
use ucdk_ledger::State;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::action::Kind as ActionKind;
use ucdk_release_proto::v1::{Reason, RowState, Verdict};
use ucdk_release_seams::{Grant, Publisher as _, SeamError};

/// A rig whose read-back reads the one package `ucdk-cli`, up to `wait_polls` more times.
fn rig(name: &str, wait_polls: u32) -> Rig {
    let mut rig = Rig::with(name, |m| {
        let smoke = common::action_mut(m, "publish", "smoke");
        if let Some(ActionKind::Smoke(s)) = &mut smoke.kind {
            s.artifacts = vec!["ucdk-cli".to_owned()];
        }
        smoke.validations[0].wait_polls = wait_polls;
    });
    rig.seal(None);
    rig.publish(&["ucdk-cli"]);
    rig
}

/// What a request for the read-back left: its verdict, its single row, the reads and pauses
/// it made, and the state of its record.
struct Seen {
    verdict: Verdict,
    row: (String, RowState, String),
    observes: u64,
    fetches: u64,
    ticks: Vec<u32>,
    state: State,
}

fn read_back(rig: &mut Rig) -> Seen {
    let outcome = rig.handle("publish", &["validation:visible"]);
    let [report] = &outcome.validations[..] else {
        panic!("one report: {outcome:?}");
    };
    let [row] = &report.rows[..] else {
        panic!("one row: {:?}", report.rows);
    };
    let (_, state) = rig.ledger.validation("publish", "visible").expect("record");
    Seen {
        verdict: outcome.verdict,
        row: (
            row.name.clone(),
            RowState::try_from(row.state).expect("a row state"),
            row.detail.clone(),
        ),
        observes: rig.registry.observes,
        fetches: rig.registry.fetches,
        ticks: rig.pause.ticks().to_vec(),
        state,
    }
}

fn passes(seen: &Seen) -> bool {
    seen.verdict == Verdict::Succeeded
        && seen.row.0 == readback_row("ucdk-cli")
        && seen.row.1 == RowState::RowPassed
        && seen.row.2.starts_with("serves ")
        && seen.state == State::Confirmed
}

fn down(why: &str) -> Option<Result<Presence, SeamError>> {
    Some(Err(SeamError::Unknown(why.to_owned())))
}

#[test]
fn an_observe_that_fails_once_is_read_again_after_one_pause() {
    let mut rig = rig("poll_observe_error", 2);
    rig.registry.observe_script = [down("the connection dropped")].into();
    let seen = read_back(&mut rig);
    assert!(passes(&seen), "{:?}", seen.row);
    assert_eq!((seen.observes, seen.fetches), (2, 1));
    assert_eq!(seen.ticks, [1]);
}

#[test]
fn a_fetch_that_fails_once_is_read_again_after_one_pause() {
    let mut rig = rig("poll_fetch_error", 2);
    rig.registry.fetch_script = [Some(Err(SeamError::Unknown("reset".to_owned())))].into();
    let seen = read_back(&mut rig);
    assert!(passes(&seen), "{:?}", seen.row);
    assert_eq!((seen.observes, seen.fetches), (2, 2));
    assert_eq!(seen.ticks, [1]);
}

#[test]
fn a_fetch_of_no_bytes_is_read_again_after_one_pause() {
    let mut rig = rig("poll_fetch_empty", 2);
    rig.registry.fetch_script = [Some(Ok(None))].into();
    let seen = read_back(&mut rig);
    assert!(passes(&seen), "{:?}", seen.row);
    assert_eq!((seen.observes, seen.fetches), (2, 2));
    assert_eq!(seen.ticks, [1]);
}

#[test]
fn a_coordinate_that_is_staged_and_then_present_passes_at_the_second_read() {
    let mut rig = rig("poll_staged", 2);
    rig.registry.observe_script = [Some(Ok(Presence::Staged(Held::default())))].into();
    let seen = read_back(&mut rig);
    assert!(passes(&seen), "{:?}", seen.row);
    assert_eq!((seen.observes, seen.fetches), (2, 1));
    assert_eq!(seen.ticks, [1]);
}

#[test]
fn with_no_polls_a_coordinate_is_read_once_and_never_waited_for() {
    let mut rig = rig("poll_none", 0);
    rig.registry.observe_script = [Some(Ok(Presence::Absent))].into();
    let seen = read_back(&mut rig);
    assert_eq!(seen.verdict, Verdict::ValidationFailed);
    assert_eq!(
        seen.row,
        (
            readback_row("ucdk-cli"),
            RowState::RowFailed,
            "the channel does not serve it".to_owned()
        )
    );
    assert_eq!((seen.observes, seen.fetches), (1, 0));
    assert_eq!(seen.ticks, Vec::<u32>::new());
    assert_eq!(seen.state, State::Intent);
}

#[test]
fn a_bound_of_n_polls_that_runs_out_is_n_plus_one_reads_and_n_pauses() {
    for n in [1_u32, 3] {
        let mut rig = rig(&format!("poll_bound_{n}"), n);
        rig.registry.observe_script = (0..=n).map(|_| Some(Ok(Presence::Absent))).collect();
        let seen = read_back(&mut rig);
        assert_eq!(seen.verdict, Verdict::ValidationFailed, "bound {n}");
        assert_eq!(seen.row.1, RowState::RowFailed, "bound {n}");
        assert_eq!(
            (seen.observes, seen.fetches),
            (u64::from(n) + 1, 0),
            "bound {n}"
        );
        assert_eq!(seen.ticks, vec![1; n as usize], "bound {n}");
        assert_eq!(seen.state, State::Intent, "bound {n}");
    }

    // A read that fails at every poll ends unreadable, not failed, after the same reads.
    let mut rig = rig("poll_bound_unreadable", 2);
    rig.registry.observe_script = (0..3).map(|_| down("down")).collect();
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Indeterminate, Reason::Channel)
    );
    assert_eq!(rig.registry.observes, 3);
    assert_eq!(rig.pause.ticks(), [1, 1]);
    assert_eq!(
        outcome.validations[0].rows[0].state,
        RowState::RowUnreadable as i32
    );
}

#[test]
fn burned_yanked_and_wrong_digest_answers_stop_the_polling_at_once() {
    type Arrange = fn(&mut Rig);
    let cases: [(&str, &str, Arrange); 4] = [
        ("burned", "the coordinate is burned", |rig| {
            rig.registry.observe_script = [Some(Ok(Presence::Burned))].into();
        }),
        ("yanked", "the coordinate is yanked", |rig| {
            rig.registry
                .inner
                .yank(&Grant::none(), &coordinate("ucdk-cli"), "withdrawn")
                .expect("yank");
        }),
        ("wrong_digest", "the channel holds digest ", |rig| {
            rig.registry
                .inner
                .present_different(&coordinate("ucdk-cli"), b"other bytes");
        }),
        (
            "wrong_bytes",
            "the bytes the channel serves hash to ",
            |rig| {
                rig.registry.fetch = Fetch::Answer(Ok(Some(b"not the member".to_vec())));
            },
        ),
    ];
    for (name, said, arrange) in cases {
        let mut rig = rig(&format!("poll_terminal_{name}"), 5);
        arrange(&mut rig);
        let seen = read_back(&mut rig);
        assert_eq!(seen.verdict, Verdict::ValidationFailed, "{name}");
        assert_eq!(seen.row.0, readback_row("ucdk-cli"), "{name}");
        assert_eq!(seen.row.1, RowState::RowFailed, "{name}");
        assert!(seen.row.2.starts_with(said), "{name}: {}", seen.row.2);
        assert_eq!(
            seen.observes, 1,
            "{name}: one read, though five polls were allowed"
        );
        assert_eq!(seen.fetches, u64::from(name == "wrong_bytes"), "{name}");
        assert_eq!(seen.ticks, Vec::<u32>::new(), "{name}");
        assert_eq!(seen.state, State::Intent, "{name}");
    }
}
