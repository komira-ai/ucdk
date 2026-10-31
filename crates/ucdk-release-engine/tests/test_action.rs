// SPDX-License-Identifier: Apache-2.0
//! Test actions: the build system run once per target, one derived row per target, and what the
//! ledger says afterwards. Every test runs the first machine of the project over the fakes; the
//! build system is a `FakeBuildSystem` that records the command it was asked to run.

mod common;

use std::collections::BTreeMap;
use std::fs;

use common::{REVISION, Rig, Scripted, Watched, test_row};
use ucdk_core::RecordPath;
use ucdk_ledger::{Ledger, Record as Row, State, Token};
use ucdk_release::action_input::{Given, action_input};
use ucdk_release::hashes::machine_hash;
use ucdk_release::machine::Checked;
use ucdk_release_fake::FakeBuildSystem;
use ucdk_release_proto::v1::{BuildSystemKind, Reason, RowState, Verdict};
use ucdk_release_seams::TestOutcome;

const SET_TEST: &str = "//release:set_test";

fn state_of(row: &ucdk_release_proto::v1::Row) -> RowState {
    RowState::try_from(row.state).expect("a row state")
}

/// The token and the records of the ledger: what a request that wrote nothing leaves as it was.
fn untouched(rig: &Rig) -> (Token, BTreeMap<RecordPath, Row>) {
    let snapshot = rig.ledger.inner.read().expect("read");
    (snapshot.token, snapshot.records)
}

#[test]
fn a_target_that_passes_with_a_test_run_is_a_pass() {
    let mut rig = Rig::new("passes");
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    assert_eq!(outcome.reason, Reason::Unspecified);
    assert_eq!(outcome.message, "");
    assert!(outcome.validations.is_empty());
    let [report] = &outcome.actions[..] else {
        panic!("one report: {:?}", outcome.actions);
    };
    assert_eq!(report.name, "test");
    assert_eq!(report.verdict, Verdict::Succeeded as i32);
    let [row] = &report.rows[..] else {
        panic!("one row: {:?}", report.rows);
    };
    assert_eq!(row.name, test_row(SET_TEST));
    assert_eq!(state_of(row), RowState::RowPassed);

    // The build system was asked once, to test the target, with the report it was given.
    let [command] = rig.build.commands() else {
        panic!("one command: {:?}", rig.build.commands());
    };
    assert_eq!(command.argv[..2], ["buck2", "test"]);
    assert_eq!(command.argv.last().map(String::as_str), Some(SET_TEST));
    let report_flag = format!(
        "--event-log={}/build-test-a1-0/events.json-lines",
        rig.scratch.path().display()
    );
    assert!(command.argv.contains(&report_flag), "{:?}", command.argv);

    // The ledger says: the action is confirmed for these inputs, the run is pinned, and the
    // lease was given back.
    let (record, state) = rig
        .ledger
        .action("build", "test")
        .expect("the action record");
    assert_eq!(state, State::Confirmed);
    let given = Given {
        revision: REVISION,
        stamp: &rig.stamp,
        set: None,
    };
    let input = action_input(&rig.machine, "build", "test", &given).expect("an input");
    assert_eq!(record.input, input);
    let (run, state) = rig.ledger.run_record().expect("the run record");
    assert_eq!(state, State::Confirmed);
    assert_eq!(run.run_id, "r1");
    assert_eq!(run.machine, machine_hash(rig.machine.machine()));
    assert_eq!(run.revision, REVISION);
    assert_eq!(run.version, "1.2.3");
    assert_eq!((run.build, run.commit.as_str()), (41, "abc123"));
    assert_eq!(rig.ledger.lease(), None);
}

#[test]
fn a_target_that_runs_nothing_is_not_a_pass() {
    let mut rig = Rig::new("runs_nothing");
    rig.build.test_runs_nothing(SET_TEST);
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::ValidationFailed);
    assert_eq!(outcome.reason, Reason::Validation);
    assert!(
        outcome.message.contains("no test ran"),
        "{}",
        outcome.message
    );
    let row = &outcome.actions[0].rows[0];
    assert_eq!(state_of(row), RowState::RowFailed);
    assert_eq!(row.detail, "no test ran");
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Intent, "a failure is not a state");

    // A build system that says it passed and ran nothing is not believed either.
    let mut rig = Rig::new("claims_pass");
    let mut scripted = Scripted(TestOutcome {
        passed: true,
        ran: 0,
        tail: "all green".to_owned(),
    });
    let outcome = rig.handle_with(&mut scripted, "build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::ValidationFailed);
    assert_eq!(outcome.actions[0].rows[0].detail, "no test ran");
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Intent);
}

#[test]
fn a_failing_target_is_validation_failed_and_later_targets_are_not_reached() {
    let mut rig = Rig::with("fails", |m| {
        common::set_targets(m, "build", "test", &["//t:a", "//t:b", "//t:c"]);
    });
    rig.build.test_fails("//t:b");
    rig.build.failure_tail("boom: assertion failed");
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::ValidationFailed);
    assert_eq!(outcome.reason, Reason::Validation);
    let rows = &outcome.actions[0].rows;
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [test_row("//t:a"), test_row("//t:b"), test_row("//t:c")]
    );
    let states: Vec<RowState> = rows.iter().map(state_of).collect();
    assert_eq!(
        states,
        [
            RowState::RowPassed,
            RowState::RowFailed,
            RowState::RowNotReached
        ]
    );
    assert!(
        rows[1].detail.contains("boom: assertion failed"),
        "{}",
        rows[1].detail
    );
    // The message names what failed and not what was never reached.
    assert!(outcome.message.contains("//t:b"), "{}", outcome.message);
    assert!(!outcome.message.contains("//t:c"), "{}", outcome.message);
    assert_eq!(
        rig.build.commands().len(),
        2,
        "the third target was never run"
    );
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Intent);

    // A nonzero exit that the seam reports as a failure is a failed row too.
    let mut rig = Rig::with("exits", |m| {
        common::set_targets(m, "build", "test", &["//t:a", "//t:b"]);
    });
    rig.build.fail_at_call(1);
    rig.build.failure_tail("exit 1");
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::ValidationFailed);
    let rows = &outcome.actions[0].rows;
    assert_eq!(state_of(&rows[0]), RowState::RowFailed);
    assert!(rows[0].detail.contains("exit 1"), "{}", rows[0].detail);
    assert_eq!(state_of(&rows[1]), RowState::RowNotReached);
}

#[test]
fn an_unstartable_build_system_is_indeterminate() {
    let mut rig = Rig::new("unstartable");
    rig.build.unstartable();
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Indeterminate);
    assert_eq!(outcome.reason, Reason::BuildFailed);
    assert!(
        outcome.message.contains("cannot be started"),
        "{}",
        outcome.message
    );
    let row = &outcome.actions[0].rows[0];
    assert_eq!(state_of(row), RowState::RowUnreadable);
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Intent);
}

#[test]
fn a_confirmed_action_with_the_same_inputs_is_skipped() {
    let mut rig = Rig::new("skipped");
    // The whole stage: both of its Test actions, in the order of the stage.
    let first = rig.handle("verify", &[]);
    assert_eq!(first.verdict, Verdict::Succeeded, "{}", first.message);
    let reported: Vec<&str> = first.actions.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(reported, ["unit", "lint"]);
    assert_eq!(rig.build.commands().len(), 2);
    let before = untouched(&rig);

    let second = rig.handle("verify", &[]);
    assert_eq!(second.verdict, Verdict::Noop, "{}", second.message);
    assert_eq!(second.reason, Reason::Unspecified);
    assert!(
        second
            .actions
            .iter()
            .all(|a| a.verdict == Verdict::Noop as i32)
    );
    assert_eq!(second.actions.len(), 2);
    assert_eq!(
        rig.build.commands().len(),
        2,
        "the build system was not asked again"
    );
    assert_eq!(untouched(&rig), before, "no record was written");
}

#[test]
fn an_only_selection_of_a_confirmed_action_with_the_same_inputs_is_noop() {
    let mut rig = Rig::new("only_noop");
    let first = rig.handle("build", &["action:test"]);
    assert_eq!(first.verdict, Verdict::Succeeded, "{}", first.message);
    let before = untouched(&rig);
    let token = rig.ledger.inner.read().expect("read").token;

    let second = rig.handle("build", &["action:test"]);
    assert_eq!(second.verdict, Verdict::Noop);
    assert_eq!(second.actions[0].verdict, Verdict::Noop as i32);
    assert!(second.actions[0].rows.is_empty());
    assert_eq!(rig.build.commands().len(), 1);
    assert_eq!(untouched(&rig), before);
    assert_eq!(rig.ledger.inner.read().expect("read").token, token);
    assert_eq!(
        rig.ledger.lease(),
        None,
        "the lease was taken and given back"
    );
}

#[test]
fn a_failed_test_action_is_run_again_and_leaves_its_record_unconfirmed() {
    let mut rig = Rig::new("run_again");
    rig.build.test_fails(SET_TEST);
    let failed = rig.handle("build", &["action:test"]);
    assert_eq!(failed.verdict, Verdict::ValidationFailed);
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Intent);

    // The same run id, a higher attempt, a build system that passes now: the action runs again.
    rig.build = FakeBuildSystem::new(BuildSystemKind::Buck2, &rig.clock);
    rig.attempt = 2;
    let again = rig.handle("build", &["action:test"]);
    assert_eq!(again.verdict, Verdict::Succeeded, "{}", again.message);
    assert_eq!(rig.build.commands().len(), 1, "it ran, and was not skipped");
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Confirmed);
}

#[test]
fn the_report_directory_is_made_per_invocation_under_scratch_and_removed() {
    let mut rig = Rig::with("report_dirs", |m| {
        common::set_targets(m, "build", "test", &["//t:a", "//t:b"]);
    });
    let scratch = rig.scratch.path().to_owned();
    let mut build = FakeBuildSystem::new(BuildSystemKind::Buck2, &rig.clock);
    let mut watched = Watched::new(&mut build, || {});
    let outcome = rig.handle_with(&mut watched, "build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    let reports = watched.reports.clone();
    drop(watched);

    let paths: Vec<&str> = reports.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(
        paths,
        [
            format!("{}/build-test-a1-0/events.json-lines", scratch.display()),
            format!("{}/build-test-a1-1/events.json-lines", scratch.display()),
        ]
    );
    assert!(
        reports.iter().all(|(_, made)| *made),
        "the directory existed while the build system ran"
    );
    assert_eq!(
        rig.scratch.entries(),
        Vec::<String>::new(),
        "and is removed"
    );

    // A directory left by a run that was killed is not read by the next one.
    let stale = scratch.join("verify-unit-a2-0");
    fs::create_dir_all(&stale).expect("make the stale directory");
    fs::write(stale.join("events.json-lines"), "stale").expect("write");
    rig.attempt = 2;
    rig.run_id = "r1".to_owned();
    let mut build = FakeBuildSystem::new(BuildSystemKind::Buck2, &rig.clock);
    let mut watched = Watched::new(&mut build, || {
        assert!(
            !stale.join("events.json-lines").exists(),
            "the stale report was removed before the invocation"
        );
    });
    rig.handle_with(&mut watched, "verify", &["action:unit"]);
    assert_eq!(rig.scratch.entries(), Vec::<String>::new());
}

#[test]
fn a_report_directory_that_cannot_be_made_is_an_unreadable_row() {
    let mut rig = Rig::new("blocked_dir");
    let blocked = rig.scratch.path().join("not-a-directory");
    fs::write(&blocked, "a file where the directory should be").expect("write");
    rig.scratch_dir = blocked;
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Indeterminate);
    let row = &outcome.actions[0].rows[0];
    assert_eq!(state_of(row), RowState::RowUnreadable);
    assert!(row.detail.contains("could not be made"), "{}", row.detail);
    assert!(rig.build.commands().is_empty(), "nothing was run");
}

#[test]
fn a_changed_flag_changes_the_input_digest() {
    let mut rig = Rig::new("flag_digest");
    assert_eq!(
        rig.handle("build", &["action:test"]).verdict,
        Verdict::Succeeded
    );

    // The same machine with one flag added to the build system, and the same ledger.
    let mut edited = common::machine();
    edited
        .build_system
        .as_mut()
        .expect("a build system")
        .flags
        .push("--another-flag".to_owned());
    let edited = Checked::read(edited).expect("a machine");
    let given = Given {
        revision: REVISION,
        stamp: &rig.stamp,
        set: None,
    };
    assert_ne!(
        action_input(&rig.machine, "build", "test", &given).expect("an input"),
        action_input(&edited, "build", "test", &given).expect("an input"),
        "a flag is in the input digest"
    );
    rig.machine = edited;
    let before = untouched(&rig);
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Refused);
    assert_eq!(outcome.reason, Reason::InputsChanged);
    assert_eq!(rig.build.commands().len(), 1, "nothing was run");
    assert_eq!(untouched(&rig), before, "nothing was written");
}

#[test]
fn a_run_record_that_differs_is_a_pin_mismatch() {
    let mut rig = Rig::new("pins");
    assert_eq!(
        rig.handle("verify", &["action:unit"]).verdict,
        Verdict::Succeeded
    );
    let before = untouched(&rig);

    // Another release version and another timestamp, for an action that was not run yet.
    rig.stamp.version = "9.9.9".to_owned();
    rig.stamp.timestamp_ms += 1;
    let outcome = rig.handle("verify", &["action:lint"]);
    assert_eq!(outcome.verdict, Verdict::Refused);
    assert_eq!(outcome.reason, Reason::PinMismatch);
    assert!(
        outcome.message.contains("release version, timestamp"),
        "{}",
        outcome.message
    );
    assert!(!outcome.message.contains("machine"), "{}", outcome.message);
    assert_eq!(untouched(&rig), before, "nothing was written");
    assert_eq!(rig.build.commands().len(), 1, "nothing was run");

    // Another revision, another run id, and another machine are named too.
    rig.stamp = common::stamp();
    rig.revision = "f".repeat(40);
    rig.run_id = "r2".to_owned();
    let mut edited = common::machine();
    edited.name = "renamed".to_owned();
    rig.machine = Checked::read(edited).expect("a machine");
    let outcome = rig.handle("verify", &["action:lint"]);
    assert_eq!(outcome.reason, Reason::PinMismatch);
    for field in ["run id", "machine", "revision"] {
        assert!(
            outcome.message.contains(field),
            "{field}: {}",
            outcome.message
        );
    }
}

#[test]
fn a_request_that_cannot_be_run_is_refused_before_any_ledger_call() {
    let mut rig = Rig::new("refused");
    let unknown = rig.handle("nowhere", &[]);
    assert_eq!(unknown.verdict, Verdict::Refused);
    assert_eq!(unknown.reason, Reason::StageUnknown);
    let no_match = rig.handle("build", &["action:nope"]);
    assert_eq!(no_match.verdict, Verdict::Refused);
    assert_eq!(no_match.reason, Reason::SelectorNoMatch);
    assert!(
        no_match.message.contains("action:test"),
        "the choices: {}",
        no_match.message
    );
    // The Build action is not one this engine runs.
    let build = rig.handle("build", &["action:build"]);
    assert_eq!(build.verdict, Verdict::Refused);
    assert_eq!(build.reason, Reason::Selector);
    assert!(
        build.message.contains("not a Test action"),
        "{}",
        build.message
    );
    assert_eq!(
        rig.ledger.calls(),
        0,
        "no lease was taken and nothing was read"
    );
    assert!(rig.build.commands().is_empty());
}

#[test]
fn what_the_codec_or_the_ledger_refuses_stops_the_request_as_a_ledger_failure() {
    // A run id the codec cannot write.
    let mut rig = Rig::new("bad_run_id");
    rig.run_id = "a/b".to_owned();
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Failed);
    assert_eq!(outcome.reason, Reason::Ledger);
    assert!(rig.build.commands().is_empty());

    // A record of the run the codec cannot read.
    let mut rig = Rig::new("corrupt_record");
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded);
    let snapshot = rig.ledger.inner.read().expect("read");
    let mut row = snapshot
        .records
        .values()
        .find(|r| r.kind.as_str() == "action")
        .expect("the action record")
        .clone();
    row.outputs.insert("not-a-key".to_owned(), "x".to_owned());
    rig.ledger
        .inner
        .write(&snapshot.token, ucdk_ledger::Change::Put(row))
        .expect("write");
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Failed);
    assert_eq!(outcome.reason, Reason::Ledger);
    assert!(outcome.message.contains("not-a-key"), "{}", outcome.message);

    // A ledger that can be reached to take the lease and not read after.
    let mut rig = Rig::new("reads_fail");
    rig.ledger.crash_at(3);
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Failed);
    assert_eq!(outcome.reason, Reason::Ledger);
    assert!(rig.build.commands().is_empty());

    // A ledger whose writes fail.
    let mut rig = Rig::new("writes_fail");
    rig.ledger.fail_writes(true);
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Failed);
    assert_eq!(outcome.reason, Reason::Ledger);
    assert!(
        rig.build.commands().is_empty(),
        "nothing ran before the intent was written"
    );
}
