// SPDX-License-Identifier: Apache-2.0
//! The crash sweep: a Test action, a closure validation and a read-back validation are each run
//! on a ledger that fails at its `n`th call, for every `n` a clean run makes, and then run again
//! as the next attempt of the same run. The ledger fails in two ways: it dies from the `n`th
//! call on (the process is gone), or the `n`th call runs on the ledger and answers an error (a
//! write that landed with its answer lost). Whatever call it failed at, the rerun ends with the
//! item confirmed, the records decode, no lease is left held, nothing the engine made is left
//! behind, and the test ran at most twice.

mod common;

use common::{Rig, decoded};
use ucdk_ledger::{Ledger, State};
use ucdk_release_proto::v1::Verdict;

/// How the ledger fails at its `n`th call.
#[derive(Debug, Clone, Copy)]
enum Mode {
    /// From the `n`th call on, nothing answers.
    Dies,
    /// The `n`th call runs, and answers that it failed.
    LosesAnswer,
}

/// What a sweep runs, and where to find its record.
struct Case {
    name: &'static str,
    stage: &'static str,
    only: [&'static str; 1],
    /// Prepares a rig: the set sealed, the packages published.
    arrange: fn(&mut Rig),
    /// The state of the record of the item.
    record: fn(&Rig) -> Option<State>,
    /// Whether the item is a Test action, which pins the run and runs the build system.
    pins: bool,
}

fn fail_at(rig: &Rig, mode: Mode, n: u64) {
    match mode {
        Mode::Dies => rig.ledger.crash_at(n),
        Mode::LosesAnswer => rig.ledger.lose_answer_at(n),
    }
}

fn sweep(case: &Case, mode: Mode) {
    let label = |n: u64| format!("{} {mode:?} at call {n}", case.name);
    // The calls of a clean run.
    let mut clean = Rig::new(&format!("sweep_clean_{}", case.name));
    (case.arrange)(&mut clean);
    let before = clean.ledger.calls();
    assert_eq!(
        clean.handle(case.stage, &case.only).verdict,
        Verdict::Succeeded
    );
    let calls = clean.ledger.calls() - before;
    assert!(
        calls > 6,
        "a run reads, writes, renews and releases: {calls}"
    );

    for n in 1..=calls {
        let mut rig = Rig::new(&format!("sweep_{}_{mode:?}_{n}", case.name));
        (case.arrange)(&mut rig);
        fail_at(&rig, mode, n);
        let failed = rig.handle(case.stage, &case.only);
        if matches!(mode, Mode::Dies) {
            // Only the last call, the release of the lease, comes after everything the run did.
            assert!(
                n == calls || failed.verdict != Verdict::Succeeded,
                "{}: a run whose ledger died did not succeed",
                label(n)
            );
        }
        if failed.verdict == Verdict::Succeeded {
            assert_eq!(
                (case.record)(&rig),
                Some(State::Confirmed),
                "{}: a run that succeeded confirmed its record",
                label(n)
            );
        }
        let tested = rig.build.commands().len();

        // The process is back. The next attempt is the same holder, so it takes the lease at
        // once even when the dead one never gave it back.
        rig.ledger.revive();
        rig.attempt = 2;
        let again = rig.handle(case.stage, &case.only);
        assert!(
            matches!(again.verdict, Verdict::Succeeded | Verdict::Noop),
            "{}: {:?} {}",
            label(n),
            again.verdict,
            again.message
        );
        assert!(!decoded(&rig.ledger).is_empty(), "{}", label(n));
        assert_eq!((case.record)(&rig), Some(State::Confirmed), "{}", label(n));
        if case.pins {
            let (_, state) = rig.ledger.run_record().expect("the run record");
            assert_eq!(state, State::Confirmed, "{}", label(n));
        }
        let total = rig.build.commands().len();
        // A Test action runs the build system once or twice; a validation never does.
        let allowed = if case.pins { 1..=2 } else { 0..=0 };
        assert!(
            total >= tested && allowed.contains(&total),
            "{}: the test ran {tested} time(s), then {total} in all",
            label(n)
        );
        assert_eq!(rig.scratch.entries(), Vec::<String>::new(), "{}", label(n));
        assert_eq!(rig.ledger.lease(), None, "{}", label(n));
    }
}

fn test_action_case() -> Case {
    Case {
        name: "test_action",
        stage: "build",
        only: ["action:test"],
        arrange: |_| {},
        record: |rig| rig.ledger.action("build", "test").map(|(_, s)| s),
        pins: true,
    }
}

fn closure_case() -> Case {
    Case {
        name: "closure",
        stage: "build",
        only: ["validation:closure"],
        arrange: |rig| {
            rig.seal(None);
        },
        record: |rig| rig.ledger.validation("build", "closure").map(|(_, s)| s),
        pins: false,
    }
}

fn readback_case() -> Case {
    Case {
        name: "readback",
        stage: "publish",
        only: ["validation:visible"],
        arrange: |rig| {
            rig.seal(None);
            rig.publish(&["ucdk-cli", "ucdk-lib"]);
        },
        record: |rig| rig.ledger.validation("publish", "visible").map(|(_, s)| s),
        pins: false,
    }
}

#[test]
fn test_action_every_n() {
    sweep(&test_action_case(), Mode::Dies);
}

#[test]
fn test_action_every_n_with_the_answer_lost() {
    sweep(&test_action_case(), Mode::LosesAnswer);
}

#[test]
fn closure_validation_every_n() {
    sweep(&closure_case(), Mode::Dies);
}

#[test]
fn closure_validation_every_n_with_the_answer_lost() {
    sweep(&closure_case(), Mode::LosesAnswer);
}

#[test]
fn readback_validation_every_n() {
    sweep(&readback_case(), Mode::Dies);
}

#[test]
fn readback_validation_every_n_with_the_answer_lost() {
    sweep(&readback_case(), Mode::LosesAnswer);
}

#[test]
fn a_run_over_a_file_ledger_survives_the_file_being_reopened() {
    let dir = common::Scratch::new("file_ledger");
    let path = dir.path().join("ledger.json");
    let mut rig = Rig::new("file_ledger_engine");
    let open = |rig: &Rig| {
        ucdk_ledger::FileLedger::open_with_clock(&path, std::sync::Arc::new(rig.clock.clone()))
            .expect("open the file ledger")
    };

    let ledger = open(&rig);
    let first = rig.handle_over(&ledger, "build", &["action:test"]);
    assert_eq!(first.verdict, Verdict::Succeeded, "{}", first.message);
    drop(ledger);

    // A new handle on the file: the records decode, the item is confirmed, the lease is given
    // back.
    let ledger = open(&rig);
    let records = decoded(&ledger);
    let states: Vec<State> = records.iter().map(|(_, s)| *s).collect();
    assert_eq!(states.len(), 2, "the run and the action: {records:?}");
    assert!(states.iter().all(|s| *s == State::Confirmed), "{states:?}");
    assert!(
        records.iter().any(|(r, _)| matches!(
            r,
            ucdk_release::codec::Record::Action(a) if a.stage == "build" && a.action == "test"
        )),
        "{records:?}"
    );
    assert!(ledger.read().expect("read").lease.is_none());

    // The same request, on the next attempt: nothing runs again.
    rig.attempt = 2;
    let second = rig.handle_over(&ledger, "build", &["action:test"]);
    assert_eq!(second.verdict, Verdict::Noop, "{}", second.message);
    assert_eq!(rig.build.commands().len(), 1, "the test ran once");
    assert!(ledger.read().expect("read").lease.is_none());
    assert_eq!(rig.scratch.entries(), Vec::<String>::new());
}
