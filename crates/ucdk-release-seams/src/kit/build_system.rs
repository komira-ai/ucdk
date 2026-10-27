// SPDX-License-Identifier: Apache-2.0
//! The build system kit: nineteen scenarios over the command a build system layers, the output
//! it locates and copies, the tests it runs and the heartbeat it forwards.

use std::collections::BTreeMap;

use ucdk_release::invocation::report_file_name;
use ucdk_release_proto::v1::{
    BuildStamp, BuildSystem as Authored, BuildSystemKind, Reason, Target,
};

use super::{Beats, Recorder, Report, Scenario, must, must_err, must_some, run_scenarios, same};
use crate::{BuildSystem, Command, Invocation, SeamError, Shape, TestOutcome, Verb, valid_rel};

/// The label of the one target the scenarios build and test.
pub const BUILD_LABEL: &str = "//pkg:out";

/// What the build system finds when it runs, so a scenario can ask for a good run or a bad one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildWorld {
    /// A build lists one regular file; a test runs once and passes.
    Normal,
    /// The build succeeds and lists no output.
    NoOutput,
    /// The build succeeds and lists two outputs.
    TwoOutputs,
    /// The build succeeds and its output holds a symlink.
    SymlinkInOutput,
    /// The build succeeds and lists one output, a directory that holds a file.
    DirectoryOutput,
    /// The build succeeds and lists one directory output with a symlink in it, below its top
    /// level.
    SymlinkInDirectory,
    /// The build exits nonzero after printing `tail` last.
    BuildFails { tail: String },
    /// The build system's binary cannot be started.
    Unstartable,
    /// A test exits 0 and its report shows nothing ran.
    TestRunsNothing,
    /// A test runs and fails.
    TestFails,
    /// A test exits 0 and writes no report.
    ReportMissing,
    /// A test exits 0 and its report cannot be parsed.
    ReportUnparseable,
    /// Every invocation reports `events` progress events.
    Progress { events: u32 },
}

/// What an implementation provides: a build system in a world, and the passing of time.
pub trait BuildSystemRig {
    type System: BuildSystem;

    /// A fresh build system for which `world` is true.
    fn system(&self, world: &BuildWorld) -> Self::System;

    /// Time passes (a different run, a different day): nothing built before may show in what
    /// is built after.
    fn later(&self);

    /// The files the output of [`BUILD_LABEL`] holds once the [`BuildWorld::Normal`] build ran
    /// for `inv`, as `copy_out` is to write them: each path below the entry, in the order
    /// `copy_out` writes it, and its bytes. The rig says what the build wrote from what it
    /// scripted, never from what the build system under test answers.
    fn output_of(&self, inv: &Invocation<'_>) -> Vec<(String, Vec<u8>)>;
}

/// The build system as authored, the target and the stamp of every scenario.
struct Case {
    system: Authored,
    target: Target,
    stamp: BuildStamp,
    kind: BuildSystemKind,
}

impl Case {
    fn new(kind: BuildSystemKind) -> Self {
        Self {
            system: Authored {
                kind: kind as i32,
                binary: "/tools/build".to_owned(),
                config_file: String::new(),
                mode: String::new(),
                flags: vec!["--base-one".to_owned(), "--base-two".to_owned()],
                env: BTreeMap::from([
                    ("X".to_owned(), "base".to_owned()),
                    ("KEEP".to_owned(), "keep".to_owned()),
                ]),
            },
            target: Target {
                label: BUILD_LABEL.to_owned(),
                flags: vec!["--target-one".to_owned(), "--target-two".to_owned()],
                env: BTreeMap::from([("X".to_owned(), "target".to_owned())]),
            },
            stamp: BuildStamp {
                version: "1.2.3".to_owned(),
                build_number: 41,
                source_commit: "abc123".to_owned(),
                timestamp_ms: 1700,
            },
            kind,
        }
    }

    fn report(&self) -> String {
        format!(
            "/scratch/t1/{}",
            report_file_name(self.kind).unwrap_or_default()
        )
    }

    fn inv<'a>(&'a self, verb: Verb, report: Option<&'a str>) -> Invocation<'a> {
        Invocation {
            system: &self.system,
            target: &self.target,
            verb,
            report,
            stamp: &self.stamp,
            revision: "0123",
            platform: "linux-x86_64",
        }
    }
}

/// Every verb a build system of `kind` has: Bazel locates an output with two more.
fn verbs_of(kind: BuildSystemKind) -> Vec<Verb> {
    if kind == BuildSystemKind::Bazel {
        vec![Verb::Build, Verb::Test, Verb::Query, Verb::Info]
    } else {
        vec![Verb::Build, Verb::Test]
    }
}

/// The command `bs` layers for `case` and `verb`; a `Test` gets its report path.
fn layered<B: BuildSystem>(bs: &B, case: &Case, verb: Verb) -> Command {
    let report = case.report();
    let report = (verb == Verb::Test).then_some(report.as_str());
    must(
        bs.command(&case.inv(verb, report)),
        &format!("command of the {verb} verb"),
    )
}

/// The place of the first argument that holds `part`.
fn index_holding(argv: &[String], part: &str) -> usize {
    must_some(
        argv.iter().position(|a| a.contains(part)),
        &format!("{part} is not in the argument vector {argv:?}"),
    )
}

fn index_of(argv: &[String], arg: &str) -> usize {
    must_some(
        argv.iter().position(|a| a == arg),
        &format!("{arg} is not in the argument vector {argv:?}"),
    )
}

/// The place of the last argument that is `arg`.
fn last_index_of(argv: &[String], arg: &str) -> usize {
    must_some(
        argv.iter().rposition(|a| a == arg),
        &format!("{arg} is not in the argument vector {argv:?}"),
    )
}

fn env_of<'a>(command: &'a Command, key: &str) -> Vec<&'a str> {
    command
        .env
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .collect()
}

/// What building the target and copying its output writes, as the entry sees it.
fn built_bytes<B: BuildSystem>(bs: &mut B, case: &Case) -> Vec<(String, Vec<u8>)> {
    let beats = Beats::counting();
    let located = must(bs.build(&case.inv(Verb::Build, None), &beats), "build");
    let first = must_some(located.first(), "the output a build lists");
    let mut entry = Recorder::default();
    must(bs.copy_out(first, &mut entry), "copy_out");
    entry.calls
}

fn test_outcome<B: BuildSystem>(
    bs: &mut B,
    case: &Case,
    beats: &Beats<'_>,
) -> Result<TestOutcome, SeamError> {
    let report = case.report();
    bs.test(&case.inv(Verb::Test, Some(&report)), beats)
}

pub fn run_build_system_kit<R: BuildSystemRig>(rig: &R) -> Report {
    run_scenarios(rig, &build_system_scenarios())
}

pub fn build_system_scenarios<R: BuildSystemRig>() -> Vec<Scenario<R>> {
    vec![
        Scenario::new("same_command_same_bytes", same_command_same_bytes::<R>),
        Scenario::new(
            "base_flags_come_before_target_flags_in_order",
            base_flags_come_before_target_flags_in_order::<R>,
        ),
        Scenario::new(
            "a_target_environment_value_replaces_the_base_value_of_the_same_key",
            a_target_environment_value_replaces_the_base_value_of_the_same_key::<R>,
        ),
        Scenario::new(
            "nothing_is_inherited_from_the_process_environment",
            nothing_is_inherited_from_the_process_environment::<R>,
        ),
        Scenario::new(
            "placeholders_are_substituted_in_flag_and_environment_values",
            placeholders_are_substituted_in_flag_and_environment_values::<R>,
        ),
        Scenario::new(
            "the_stamp_reaches_a_build_only_through_placeholders",
            the_stamp_reaches_a_build_only_through_placeholders::<R>,
        ),
        Scenario::new(
            "ucdk_owned_flags_come_after_the_authors",
            ucdk_owned_flags_come_after_the_authors::<R>,
        ),
        Scenario::new(
            "a_build_lists_exactly_one_output",
            a_build_lists_exactly_one_output::<R>,
        ),
        Scenario::new("no_output_is_build_output", no_output_is_build_output::<R>),
        Scenario::new(
            "two_outputs_are_build_output",
            two_outputs_are_build_output::<R>,
        ),
        Scenario::new(
            "a_symlink_in_an_output_is_refused",
            a_symlink_in_an_output_is_refused::<R>,
        ),
        Scenario::new(
            "a_failed_build_is_permanent_with_the_tail_of_its_output",
            a_failed_build_is_permanent_with_the_tail_of_its_output::<R>,
        ),
        Scenario::new("unstartable_is_not_failed", unstartable_is_not_failed::<R>),
        Scenario::new(
            "nothing_run_specific_in_the_bytes",
            nothing_run_specific_in_the_bytes::<R>,
        ),
        Scenario::new(
            "copy_out_writes_only_into_the_entry_it_is_given",
            copy_out_writes_only_into_the_entry_it_is_given::<R>,
        ),
        Scenario::new(
            "a_test_target_that_runs_nothing_is_not_a_pass",
            a_test_target_that_runs_nothing_is_not_a_pass::<R>,
        ),
        Scenario::new(
            "a_failing_test_target_is_not_a_pass",
            a_failing_test_target_is_not_a_pass::<R>,
        ),
        Scenario::new(
            "a_test_report_that_cannot_be_read_is_not_a_pass",
            a_test_report_that_cannot_be_read_is_not_a_pass::<R>,
        ),
        Scenario::new(
            "the_build_system_beats_on_progress_and_stops_when_told",
            the_build_system_beats_on_progress_and_stops_when_told::<R>,
        ),
    ]
}

fn same_command_same_bytes<R: BuildSystemRig>(rig: &R) {
    let mut bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    let first = built_bytes(&mut bs, &case);
    let second = built_bytes(&mut bs, &case);
    same(
        first.is_empty(),
        false,
        "a build that wrote nothing proves nothing",
    );
    let wrong: Vec<_> = rig
        .output_of(&case.inv(Verb::Build, None))
        .into_iter()
        .filter(|file| !first.contains(file))
        .collect();
    same(
        wrong.is_empty(),
        true,
        &format!("files of the output that copy_out did not write with their bytes: {wrong:?}"),
    );
    same(second, first, "the bytes of two builds of one command");
}

fn base_flags_come_before_target_flags_in_order<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    for verb in verbs_of(bs.kind()) {
        let command = layered(&bs, &case, verb);
        let at: Vec<usize> = ["--base-one", "--base-two", "--target-one", "--target-two"]
            .iter()
            .map(|flag| index_of(&command.argv, flag))
            .collect();
        same(
            at.windows(2).all(|w| w[0] < w[1]),
            true,
            &format!("the flags of the {verb} verb in the order base then target"),
        );
        // Each flag an author wrote is passed once, not again further on.
        for flag in ["--base-one", "--base-two", "--target-one", "--target-two"] {
            same(
                command.argv.iter().filter(|a| *a == flag).count(),
                1,
                &format!("the times {flag} is in the argument vector of the {verb} verb"),
            );
        }
    }
}

fn a_target_environment_value_replaces_the_base_value_of_the_same_key<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    for verb in verbs_of(bs.kind()) {
        let command = layered(&bs, &case, verb);
        same(
            env_of(&command, "X"),
            vec!["target"],
            &format!("the {verb} verb's value of a key both authored"),
        );
        same(
            env_of(&command, "KEEP"),
            vec!["keep"],
            &format!("the {verb} verb's value of a key only the base authored"),
        );
    }
}

fn nothing_is_inherited_from_the_process_environment<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    for verb in verbs_of(bs.kind()) {
        let command = layered(&bs, &case, verb);
        let keys: Vec<&str> = command.env.iter().map(|(k, _)| k.as_str()).collect();
        same(
            keys,
            vec!["KEEP", "X"],
            &format!("the keys of the {verb} verb's environment"),
        );
    }
}

fn placeholders_are_substituted_in_flag_and_environment_values<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let mut case = Case::new(bs.kind());
    case.system.flags = vec!["--v={release_version}-{build_number}".to_owned()];
    case.target.flags = vec![
        "--c={source_commit}".to_owned(),
        "--r={revision}@{platform}".to_owned(),
        "--t={timestamp_ms}".to_owned(),
    ];
    case.target.env = BTreeMap::from([("STAMP".to_owned(), "{release_version}".to_owned())]);
    for verb in verbs_of(bs.kind()) {
        let command = layered(&bs, &case, verb);
        for flag in [
            "--v=1.2.3-41",
            "--c=abc123",
            "--r=0123@linux-x86_64",
            "--t=1700",
        ] {
            index_of(&command.argv, flag);
        }
        same(
            env_of(&command, "STAMP"),
            vec!["1.2.3"],
            &format!("a placeholder in an environment value of the {verb} verb"),
        );
        same(
            command.argv.iter().any(|a| a.contains('{')),
            false,
            &format!("a placeholder left in the argument vector of the {verb} verb"),
        );
    }
}

fn the_stamp_reaches_a_build_only_through_placeholders<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let mut case = Case::new(bs.kind());
    let verbs = verbs_of(bs.kind());
    let before: Vec<Command> = verbs.iter().map(|v| layered(&bs, &case, *v)).collect();
    case.stamp = BuildStamp {
        version: "9.9.9".to_owned(),
        build_number: 7,
        source_commit: "ffffff".to_owned(),
        timestamp_ms: 1,
    };
    for (verb, before) in verbs.iter().zip(before) {
        same(
            layered(&bs, &case, *verb),
            before,
            &format!("the {verb} command when only the stamp changed and no flag names it"),
        );
    }
}

fn ucdk_owned_flags_come_after_the_authors<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    let report = case.report();
    let command = must(bs.command(&case.inv(Verb::Test, Some(&report))), "command");
    let owned = index_holding(&command.argv, &report);
    let last_author = ["--base-one", "--base-two", "--target-one", "--target-two"]
        .iter()
        .map(|flag| last_index_of(&command.argv, flag))
        .max()
        .unwrap_or_default();
    same(
        owned > last_author,
        true,
        "the flag ucdk owns after the last flag an author wrote",
    );
}

fn a_build_lists_exactly_one_output<R: BuildSystemRig>(rig: &R) {
    for (world, shape) in [
        (BuildWorld::Normal, Shape::File),
        (BuildWorld::DirectoryOutput, Shape::Directory),
    ] {
        let mut bs = rig.system(&world);
        let case = Case::new(bs.kind());
        let beats = Beats::counting();
        let located = must(
            bs.build(&case.inv(Verb::Build, None), &beats),
            &format!("build in {world:?}"),
        );
        same(
            located.len(),
            1,
            &format!("the outputs of a build in {world:?}"),
        );
        same(
            located[0].label.as_str(),
            BUILD_LABEL,
            "the label of the output",
        );
        same(located[0].path.starts_with('/'), true, "an absolute path");
        same(
            located[0].shape,
            shape,
            &format!("the shape of the output in {world:?}"),
        );
    }
}

/// A build in `world` that must be refused as `BUILD_OUTPUT`.
fn refused_as_build_output<R: BuildSystemRig>(rig: &R, world: &BuildWorld) {
    let mut bs = rig.system(world);
    let case = Case::new(bs.kind());
    let beats = Beats::counting();
    let refusal = must_err(bs.build(&case.inv(Verb::Build, None), &beats), "build");
    same(
        refusal,
        SeamError::Refused(Reason::BuildOutput),
        "the answer to a bad output",
    );
}

fn no_output_is_build_output<R: BuildSystemRig>(rig: &R) {
    refused_as_build_output(rig, &BuildWorld::NoOutput);
}

fn two_outputs_are_build_output<R: BuildSystemRig>(rig: &R) {
    refused_as_build_output(rig, &BuildWorld::TwoOutputs);
}

fn a_symlink_in_an_output_is_refused<R: BuildSystemRig>(rig: &R) {
    // The output is a symlink itself, or a directory with one below its top level.
    refused_as_build_output(rig, &BuildWorld::SymlinkInOutput);
    refused_as_build_output(rig, &BuildWorld::SymlinkInDirectory);
}

fn a_failed_build_is_permanent_with_the_tail_of_its_output<R: BuildSystemRig>(rig: &R) {
    let tail = "error: boom 42".to_owned();
    let mut bs = rig.system(&BuildWorld::BuildFails { tail: tail.clone() });
    let case = Case::new(bs.kind());
    let beats = Beats::counting();
    let failure = must_err(bs.build(&case.inv(Verb::Build, None), &beats), "build");
    same(
        matches!(&failure, SeamError::Permanent(why) if why.contains(&tail)),
        true,
        &format!("a failed build is Permanent and names the tail, got {failure:?}"),
    );
}

fn unstartable_is_not_failed<R: BuildSystemRig>(rig: &R) {
    let mut bs = rig.system(&BuildWorld::Unstartable);
    let case = Case::new(bs.kind());
    let beats = Beats::counting();
    let failure = must_err(bs.build(&case.inv(Verb::Build, None), &beats), "build");
    same(
        matches!(failure, SeamError::Unknown(_)),
        true,
        "a build system that cannot start is Unknown, not a failed build",
    );
}

fn nothing_run_specific_in_the_bytes<R: BuildSystemRig>(rig: &R) {
    let mut first = rig.system(&BuildWorld::Normal);
    let case = Case::new(first.kind());
    let before = built_bytes(&mut first, &case);
    rig.later();
    let mut second = rig.system(&BuildWorld::Normal);
    let after = built_bytes(&mut second, &case);
    same(
        after,
        before,
        "the bytes of the same build in another run, later",
    );
}

fn copy_out_writes_only_into_the_entry_it_is_given<R: BuildSystemRig>(rig: &R) {
    let mut bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    let written = built_bytes(&mut bs, &case);
    same(written.is_empty(), false, "copy_out wrote nothing");
    same(
        written
            .iter()
            .all(|(rel, _)| valid_rel(rel) && !rel.contains('\\')),
        true,
        &format!("every path copy_out wrote is a relative path of the entry: {written:?}"),
    );
    let wanted: Vec<String> = rig
        .output_of(&case.inv(Verb::Build, None))
        .into_iter()
        .map(|(rel, _)| rel)
        .collect();
    same(
        written.into_iter().map(|(rel, _)| rel).collect::<Vec<_>>(),
        wanted,
        "the paths copy_out wrote, which are those of the output and no other",
    );
    // A directory output is copied into the entry as well, and as carefully.
    let mut bs = rig.system(&BuildWorld::DirectoryOutput);
    let case = Case::new(bs.kind());
    let written = built_bytes(&mut bs, &case);
    same(
        written.is_empty(),
        false,
        "copy_out of a directory wrote nothing",
    );
    same(
        written
            .iter()
            .all(|(rel, _)| valid_rel(rel) && !rel.contains('\\')),
        true,
        &format!("every path copy_out wrote of a directory is a relative path: {written:?}"),
    );
}

fn a_test_target_that_runs_nothing_is_not_a_pass<R: BuildSystemRig>(rig: &R) {
    let mut good = rig.system(&BuildWorld::Normal);
    let case = Case::new(good.kind());
    let passing = must(
        test_outcome(&mut good, &case, &Beats::counting()),
        "a test that runs",
    );
    same(
        (passing.passed, passing.ran >= 1),
        (true, true),
        "a test that ran once and passed",
    );
    let mut bs = rig.system(&BuildWorld::TestRunsNothing);
    let outcome = must(test_outcome(&mut bs, &case, &Beats::counting()), "test");
    same(
        (outcome.passed, outcome.ran),
        (false, 0),
        "a test that ran nothing",
    );
}

fn a_failing_test_target_is_not_a_pass<R: BuildSystemRig>(rig: &R) {
    let mut bs = rig.system(&BuildWorld::TestFails);
    let case = Case::new(bs.kind());
    let outcome = must(test_outcome(&mut bs, &case, &Beats::counting()), "test");
    same(outcome.passed, false, "a test that failed");
    same(outcome.ran >= 1, true, "a failing test still ran");
    same(outcome.tail.is_empty(), false, "the tail of a failing test");
}

fn a_test_report_that_cannot_be_read_is_not_a_pass<R: BuildSystemRig>(rig: &R) {
    for world in [BuildWorld::ReportMissing, BuildWorld::ReportUnparseable] {
        let mut bs = rig.system(&world);
        let case = Case::new(bs.kind());
        let outcome = must(test_outcome(&mut bs, &case, &Beats::counting()), "test");
        same(
            (outcome.passed, outcome.ran),
            (false, 0),
            &format!("a test whose report is unreadable ({world:?})"),
        );
    }
}

fn the_build_system_beats_on_progress_and_stops_when_told<R: BuildSystemRig>(rig: &R) {
    let world = BuildWorld::Progress { events: 3 };
    let mut bs = rig.system(&world);
    let case = Case::new(bs.kind());
    let counting = Beats::counting();
    must(bs.build(&case.inv(Verb::Build, None), &counting), "build");
    same(
        counting.count() >= 3,
        true,
        "the beats of a build with three progress events",
    );
    let counting = Beats::counting();
    must(test_outcome(&mut bs, &case, &counting), "test");
    same(
        counting.count() >= 3,
        true,
        "the beats of a test with three progress events",
    );
    let stopping = Beats::stopping_at(2);
    let build = must_err(
        bs.build(&case.inv(Verb::Build, None), &stopping),
        "a build that is stopped",
    );
    same(
        build,
        SeamError::Refused(Reason::LeaseLost),
        "a build told to stop",
    );
    let stopping = Beats::stopping_at(2);
    let test = must_err(
        test_outcome(&mut bs, &case, &stopping),
        "a test that is stopped",
    );
    same(
        test,
        SeamError::Refused(Reason::LeaseLost),
        "a test told to stop",
    );
}
