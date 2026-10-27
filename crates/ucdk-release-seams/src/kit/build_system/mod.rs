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

mod layering;

use layering::{
    a_target_environment_value_replaces_the_base_value_of_the_same_key,
    base_flags_come_before_target_flags_in_order,
    nothing_is_inherited_from_the_process_environment,
    placeholders_are_substituted_in_flag_and_environment_values,
    the_stamp_reaches_a_build_only_through_placeholders, ucdk_owned_flags_come_after_the_authors,
};

/// The label of the one target the scenarios build and test.
pub const BUILD_LABEL: &str = "//pkg:out";

/// What the build system finds when it runs, so a scenario can ask for a good run or a bad one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildWorld {
    /// A build lists one regular file at a path that holds a space; a test runs once and
    /// passes.
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
    /// The build succeeds and its listing names another target than the one built.
    ListsAnotherTarget,
    /// The build succeeds and its listing names `path` as the output of the target built, a
    /// path that is relative, climbs with `..` or holds a control character. The rig writes it
    /// in the form its kind's listing carries a path in.
    ListsAHostilePath { path: String },
    /// The build exits nonzero after printing `head` first and `tail` last.
    BuildFails { head: String, tail: String },
    /// The build system's binary cannot be started.
    Unstartable,
    /// A test exits 0 and its report shows nothing ran.
    TestRunsNothing,
    /// A test exits 0 and its report holds results that are all skipped or untested.
    TestSkipsEverything,
    /// A test runs once and fails, printing `tail` last.
    TestFails { tail: String },
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

    /// The files the output of [`BUILD_LABEL`] holds once the build of `world` ran for `inv`, as
    /// `copy_out` is to write them: each path below the entry, in the order `copy_out` writes
    /// it, and its bytes. `world` is [`BuildWorld::Normal`] (a file) or
    /// [`BuildWorld::DirectoryOutput`] (a directory). The rig says what the build wrote from
    /// what it scripted, never from what the build system under test answers.
    fn output_of(&self, inv: &Invocation<'_>, world: &BuildWorld) -> Vec<(String, Vec<u8>)>;

    /// The path the build of [`BUILD_LABEL`] lists in `world`, [`BuildWorld::Normal`] (a file) or
    /// [`BuildWorld::DirectoryOutput`] (a directory), as the rig scripted it. It holds a space.
    fn listed_path(&self, world: &BuildWorld) -> String;

    /// Every command `system` ran so far, in order, as the process under it was given it: the
    /// argument vector and the environment of each child; a Bazel build's `Query` and `Info` are
    /// among them, in the order they ran. A command that could not be layered is
    /// never run, so it is not here.
    fn executed(&self, system: &Self::System) -> Vec<Command>;
}

/// The worlds in which a build lists an output that `copy_out` copies: a file and a directory.
const COPIED: [BuildWorld; 2] = [BuildWorld::Normal, BuildWorld::DirectoryOutput];

/// The build system as authored, the target and the stamp of every scenario.
struct Case {
    system: Authored,
    target: Target,
    stamp: BuildStamp,
    revision: String,
    platform: String,
    kind: BuildSystemKind,
}

impl Case {
    fn new(kind: BuildSystemKind) -> Self {
        Self {
            system: Authored {
                kind: kind as i32,
                binary: "/tools/build".to_owned(),
                config_file: "/cfg/build.rc".to_owned(),
                mode: "opt".to_owned(),
                flags: vec![
                    "--base-one".to_owned(),
                    "--base-two".to_owned(),
                    "--shared".to_owned(),
                ],
                env: BTreeMap::from([
                    ("X".to_owned(), "base".to_owned()),
                    ("KEEP".to_owned(), "keep".to_owned()),
                ]),
            },
            target: Target {
                label: BUILD_LABEL.to_owned(),
                flags: vec![
                    "--target-one".to_owned(),
                    "--target-two".to_owned(),
                    "--shared".to_owned(),
                ],
                env: BTreeMap::from([("X".to_owned(), "target".to_owned())]),
            },
            stamp: BuildStamp {
                version: "1.2.3".to_owned(),
                build_number: 41,
                source_commit: "abc123".to_owned(),
                timestamp_ms: 1700,
            },
            revision: "0123".to_owned(),
            platform: "linux-x86_64".to_owned(),
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
            revision: &self.revision,
            platform: &self.platform,
        }
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
    for world in &COPIED {
        let mut bs = rig.system(world);
        let case = Case::new(bs.kind());
        let first = built_bytes(&mut bs, &case);
        let second = built_bytes(&mut bs, &case);
        same(
            first.is_empty(),
            false,
            &format!("a build that wrote nothing proves nothing ({world:?})"),
        );
        let wrong: Vec<_> = rig
            .output_of(&case.inv(Verb::Build, None), world)
            .into_iter()
            .filter(|file| !first.contains(file))
            .collect();
        same(
            wrong.is_empty(),
            true,
            &format!(
                "files of the output ({world:?}) that copy_out did not write with their bytes: \
                 {wrong:?}"
            ),
        );
        same(
            second,
            first.clone(),
            &format!("the bytes of two builds of one command ({world:?})"),
        );
        // Another command in the same system: the bytes follow the command, never the first
        // build's. A flag, a stamp that a flag names and an environment value all change.
        let mut other = Case::new(bs.kind());
        other.system.flags.push("--n={build_number}".to_owned());
        other.stamp.build_number = 99;
        other.target.env.insert("Y".to_owned(), "two".to_owned());
        let changed = built_bytes(&mut bs, &other);
        same(
            changed == first,
            false,
            &format!("a build of another command gave the bytes of the first ({world:?})"),
        );
        let wrong: Vec<_> = rig
            .output_of(&other.inv(Verb::Build, None), world)
            .into_iter()
            .filter(|file| !changed.contains(file))
            .collect();
        same(
            wrong.is_empty(),
            true,
            &format!(
                "files of the output ({world:?}) of another command that copy_out did not write \
                 with their bytes: {wrong:?}"
            ),
        );
    }
    what_ran_is_what_was_layered(rig);
}

/// What a build of one artifact runs, in order: a `Build`, and for Bazel the `Query` and then
/// the `Info` that locate its output. A test of one target runs a `Test` and nothing else.
fn build_verbs(kind: BuildSystemKind) -> Vec<Verb> {
    if kind == BuildSystemKind::Bazel {
        vec![Verb::Build, Verb::Query, Verb::Info]
    } else {
        vec![Verb::Build]
    }
}

/// The commands a call added to what `bs` ran must be exactly the commands the build system
/// layers for `verbs`, each once and in that order: a command run twice or never, one that is
/// not the one layered, or a verb that runs another's, is found here. A command is compared
/// whole, so every verb the kind has is held. The layering the build system gives is held
/// against the contract's by the scenarios that read `command`, so this holds that what ran is
/// what was layered, whichever `command` the implementation has.
fn ran_as_layered<R: BuildSystemRig>(
    rig: &R,
    bs: &R::System,
    before: usize,
    expected: &[Verb],
    case: &Case,
    what: &str,
) {
    let want: Vec<Command> = expected.iter().map(|v| layered(bs, case, *v)).collect();
    same(
        rig.executed(bs)[before..].to_vec(),
        want,
        &format!("the commands {what} ran, which are exactly {expected:?}, each once and in order"),
    );
}

fn what_ran_is_what_was_layered<R: BuildSystemRig>(rig: &R) {
    let mut bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    let before = rig.executed(&bs).len();
    must(
        bs.build(&case.inv(Verb::Build, None), &Beats::counting()),
        "build",
    );
    ran_as_layered(rig, &bs, before, &build_verbs(bs.kind()), &case, "a build");
    let before = rig.executed(&bs).len();
    must(test_outcome(&mut bs, &case, &Beats::counting()), "test");
    ran_as_layered(rig, &bs, before, &[Verb::Test], &case, "a test");
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
        // The path holds a space, and is the whole of what follows the label: the listing is
        // split at the first space, not the last.
        let listed = rig.listed_path(&world);
        same(
            listed.contains(' '),
            true,
            "the path the rig scripts holds a space",
        );
        same(
            located[0].path.as_str(),
            listed.as_str(),
            &format!("the path of the output in {world:?}"),
        );
        same(
            located[0].shape,
            shape,
            &format!("the shape of the output in {world:?}"),
        );
    }
    // A listing that names another target is not the output of this one, and a path that is
    // not absolute, climbs or holds a control character is not a path a build wrote: each is
    // refused, whatever the line says first.
    refused_as_build_output(rig, &BuildWorld::ListsAnotherTarget);
    for path in HOSTILE_PATHS {
        refused_as_build_output(
            rig,
            &BuildWorld::ListsAHostilePath {
                path: path.to_owned(),
            },
        );
    }
}

/// Paths a listing must not be read as: with a tab, with another control character, relative,
/// and climbing.
const HOSTILE_PATHS: [&str; 4] = [
    "/out/with\ttab/out",
    "/out/with\u{7}bell/out",
    "out/relative/out",
    "/out/../escape/out",
];

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
    // The output is long: a head the answer must not carry, and the tail it must.
    let first = "noise line 0 of the head";
    let head: String = (0..20_000)
        .map(|i| format!("noise line {i} of the head\n"))
        .collect();
    let tail = "error: boom 42".to_owned();
    let mut bs = rig.system(&BuildWorld::BuildFails {
        head,
        tail: tail.clone(),
    });
    let case = Case::new(bs.kind());
    let beats = Beats::counting();
    let failure = must_err(bs.build(&case.inv(Verb::Build, None), &beats), "build");
    let text = failure.to_string();
    same(
        matches!(&failure, SeamError::Permanent(_)) & text.contains(&tail),
        true,
        &format!("a failed build is Permanent and names the tail, got {failure:?}"),
    );
    same(
        text.contains(first),
        false,
        "the answer to a failed build holds the last lines of the output, not the first",
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
    let outcome = must_err(test_outcome(&mut bs, &case, &beats), "test");
    same(
        matches!(outcome, SeamError::Unknown(_)),
        true,
        "a test of a build system that cannot start is Unknown, not a failed test",
    );
}

fn nothing_run_specific_in_the_bytes<R: BuildSystemRig>(rig: &R) {
    for world in &COPIED {
        let mut first = rig.system(world);
        let case = Case::new(first.kind());
        let before = built_bytes(&mut first, &case);
        rig.later();
        let mut second = rig.system(world);
        let after = built_bytes(&mut second, &case);
        same(
            after,
            before,
            &format!("the bytes of the same build ({world:?}) in another run, later"),
        );
    }
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
        .output_of(&case.inv(Verb::Build, None), &BuildWorld::Normal)
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
    let wanted: Vec<String> = rig
        .output_of(&case.inv(Verb::Build, None), &BuildWorld::DirectoryOutput)
        .into_iter()
        .map(|(rel, _)| rel)
        .collect();
    same(
        written.into_iter().map(|(rel, _)| rel).collect::<Vec<_>>(),
        wanted,
        "the paths copy_out wrote of a directory, which are those of the output and no other",
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
        (passing.passed, passing.ran),
        (true, 1),
        "a test that ran once and passed",
    );
    // Nothing ran: no report line, or results that are all skipped or untested.
    for world in [BuildWorld::TestRunsNothing, BuildWorld::TestSkipsEverything] {
        let mut bs = rig.system(&world);
        let outcome = must(test_outcome(&mut bs, &case, &Beats::counting()), "test");
        same(
            (outcome.passed, outcome.ran),
            (false, 0),
            &format!("a test that ran nothing ({world:?})"),
        );
    }
}

fn a_failing_test_target_is_not_a_pass<R: BuildSystemRig>(rig: &R) {
    let tail = "FAILED //pkg:out, 1 of 3 cases".to_owned();
    let mut bs = rig.system(&BuildWorld::TestFails { tail: tail.clone() });
    let case = Case::new(bs.kind());
    let outcome = must(test_outcome(&mut bs, &case, &Beats::counting()), "test");
    same(
        (outcome.passed, outcome.ran),
        (false, 1),
        "a test that ran once and failed",
    );
    same(
        outcome.tail.contains(&tail),
        true,
        &format!("the tail of a failing test is what it printed last, got {outcome:?}"),
    );
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
    const EVENTS: u32 = 40;
    let world = BuildWorld::Progress { events: EVENTS };
    let mut bs = rig.system(&world);
    let case = Case::new(bs.kind());
    // One beat at least for each progress event, however many there are.
    let counting = Beats::counting();
    must(bs.build(&case.inv(Verb::Build, None), &counting), "build");
    same(
        counting.count() >= EVENTS,
        true,
        "the beats of a build with many progress events",
    );
    let counting = Beats::counting();
    must(test_outcome(&mut bs, &case, &counting), "test");
    same(
        counting.count() >= EVENTS,
        true,
        "the beats of a test with many progress events",
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
    // Told to stop, it stops: no beat comes after the one that was answered with a Stop.
    same(stopping.count(), 2, "the beats of a build up to its Stop");
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
    same(stopping.count(), 2, "the beats of a test up to its Stop");
}
