// SPDX-License-Identifier: Apache-2.0
//! The build system a Test action runs through: Buck2 or Bazel as the machine names it, refused
//! before the lease when the build system given is of another kind; a report that is missing,
//! that does not parse or that shows only skipped tests is never a pass; and the report
//! directories of engines that share a scratch directory never meet.

mod common;

use std::path::Path;
use std::sync::{Arc, Barrier};

use common::{Rig, Watched};
use ucdk_ledger::State;
use ucdk_release::machine::Checked;
use ucdk_release_fake::FakeBuildSystem;
use ucdk_release_proto::v1::{BuildSystemKind, Reason, RowState, Verdict};

const SET_TEST: &str = "//release:set_test";

/// A rig whose machine and build system are Bazel.
fn bazel(name: &str) -> Rig {
    let mut rig = Rig::with(name, |m| {
        let system = m.build_system.as_mut().expect("a build system");
        system.kind = BuildSystemKind::Bazel as i32;
        system.binary = "bazel".to_owned();
        system.config_file = String::new();
        system.mode = String::new();
    });
    rig.build = FakeBuildSystem::new(BuildSystemKind::Bazel, &rig.clock);
    rig
}

/// A fault done to the build system before it is asked.
type Fault = fn(&mut FakeBuildSystem);

#[test]
fn a_bazel_machine_runs_its_targets_through_bazel_with_a_bazel_report() {
    let mut rig = bazel("bazel");
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    let [command] = rig.build.commands() else {
        panic!("one command: {:?}", rig.build.commands());
    };
    assert_eq!(command.argv[..2], ["bazel", "test"], "{:?}", command.argv);
    assert_eq!(command.argv.last().map(String::as_str), Some(SET_TEST));
    let scratch = rig.scratch.path().display().to_string();
    let report = command
        .argv
        .iter()
        .find(|a| a.contains(&scratch))
        .unwrap_or_else(|| panic!("a report under the scratch directory: {:?}", command.argv));
    assert!(report.ends_with("/events.json"), "{report}");
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Confirmed);
}

#[test]
fn a_build_system_of_another_kind_than_the_machines_is_refused_before_any_call() {
    for (name, mut rig) in [
        ("kind_bazel_given", {
            let mut rig = Rig::new("kind_bazel_given");
            rig.build = FakeBuildSystem::new(BuildSystemKind::Bazel, &rig.clock);
            rig
        }),
        ("kind_buck2_given", {
            let mut rig = bazel("kind_buck2_given");
            rig.build = FakeBuildSystem::new(BuildSystemKind::Buck2, &rig.clock);
            rig
        }),
    ] {
        let outcome = rig.handle("build", &["action:test"]);
        assert_eq!(
            (outcome.verdict, outcome.reason),
            (Verdict::Refused, Reason::BuildFailed),
            "{name}: {}",
            outcome.message
        );
        assert!(
            outcome.message.starts_with("the build system given is "),
            "{name}: {}",
            outcome.message
        );
        assert_eq!(rig.ledger.calls(), 0, "{name}: the ledger was not called");
        assert!(rig.build.commands().is_empty(), "{name}: nothing was run");
    }
    // A checked machine always names a build system (rule `machine.build_system_required`):
    // the machine read without one is refused before any engine exists.
    let mut machine = common::machine();
    machine.build_system = None;
    assert!(Checked::read(machine).is_err());
}

#[test]
fn a_report_that_is_missing_unparseable_or_only_skipped_never_confirms_the_action() {
    let faults: [(&str, Fault); 3] = [
        ("report_missing", |b| b.report_missing(SET_TEST)),
        ("report_unparseable", |b| b.report_unparseable(SET_TEST)),
        ("report_skipped", |b| b.report_skipped_only(SET_TEST)),
    ];
    for (name, fault) in faults {
        for kind in [BuildSystemKind::Buck2, BuildSystemKind::Bazel] {
            let mut rig = if kind == BuildSystemKind::Bazel {
                bazel(name)
            } else {
                Rig::new(name)
            };
            fault(&mut rig.build);
            let outcome = rig.handle("build", &["action:test"]);
            assert_ne!(outcome.verdict, Verdict::Succeeded, "{name} {kind:?}");
            let row = &outcome.actions[0].rows[0];
            assert_ne!(
                RowState::try_from(row.state).expect("a row state"),
                RowState::RowPassed,
                "{name} {kind:?}"
            );
            let (_, state) = rig.ledger.action("build", "test").expect("the record");
            assert_eq!(state, State::Intent, "{name} {kind:?}: not confirmed");
        }
    }
}

#[test]
fn engines_sharing_a_scratch_directory_never_share_a_report_directory() {
    // Two runs on two ledgers, with the same run id, stage, action and attempt: the names they
    // derive are the same, and the exclusive create keeps them apart.
    let mut first = Rig::new("shared_scratch");
    let mut second = Rig::new("shared_scratch_other");
    second.scratch_dir = first.scratch.path().to_owned();
    let both = Arc::new(Barrier::new(2));
    let mut seen = Vec::new();
    std::thread::scope(|scope| {
        let runs: Vec<_> = [&mut first, &mut second]
            .into_iter()
            .map(|rig| {
                let both = Arc::clone(&both);
                scope.spawn(move || {
                    let mut build = FakeBuildSystem::new(BuildSystemKind::Buck2, &rig.clock);
                    // Both invocations are in flight, each in its directory, at the barrier.
                    let mut watched = Watched::new(&mut build, move || {
                        both.wait();
                    });
                    let outcome = rig.handle_with(&mut watched, "build", &["action:test"]);
                    (outcome.verdict, watched.reports.clone())
                })
            })
            .collect();
        for run in runs {
            seen.push(run.join().expect("the run did not panic"));
        }
    });
    let dirs: Vec<String> = seen
        .iter()
        .map(|(verdict, reports)| {
            assert_eq!(*verdict, Verdict::Succeeded);
            let [(report, made)] = &reports[..] else {
                panic!("one invocation: {reports:?}");
            };
            assert!(made, "the directory existed while the build system ran");
            let dir = Path::new(report).parent().expect("a directory");
            assert_eq!(dir.parent(), Some(first.scratch.path()));
            dir.display().to_string()
        })
        .collect();
    assert_ne!(dirs[0], dirs[1], "each invocation had its own directory");
    assert_eq!(
        first.scratch.entries(),
        Vec::<String>::new(),
        "both were removed"
    );
}
