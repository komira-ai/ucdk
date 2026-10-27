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

    /// Every command `system` ran so far, in order, as the process under it was given it: the
    /// argument vector and the environment of each child. A command that could not be layered is
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

/// The flags the default case's authors wrote, in the order they must come: the base's, then the
/// target's. `--shared` is written by both, so it is passed twice.
const AUTHORED: [&str; 6] = [
    "--base-one",
    "--base-two",
    "--shared",
    "--target-one",
    "--target-two",
    "--shared",
];

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

/// The layering the contract gives `case` and `verb`: the function every build system's
/// `command` stands for, which a scenario compares the part it holds against.
fn contract(case: &Case, verb: Verb) -> Command {
    let report = case.report();
    let report = (verb == Verb::Test).then_some(report.as_str());
    must(
        ucdk_release::invocation::command(&case.inv(verb, report))
            .map_err(|e| SeamError::Permanent(e.to_string())),
        &format!("the contract's command of the {verb} verb"),
    )
}

/// What comes before the first flag an author wrote and what comes after the last: the binary,
/// the startup arguments, the verb word and the mode, and the flags ucdk owns with the tail of
/// the verb.
fn around_authors(argv: &[String]) -> (Vec<String>, Vec<String>) {
    let is_authored = |a: &String| AUTHORED.contains(&a.as_str());
    let first = must_some(
        argv.iter().position(is_authored),
        &format!("no flag of an author in the argument vector {argv:?}"),
    );
    let last = must_some(
        argv.iter().rposition(is_authored),
        &format!("no flag of an author in the argument vector {argv:?}"),
    );
    (argv[..first].to_vec(), argv[last + 1..].to_vec())
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
            first,
            &format!("the bytes of two builds of one command ({world:?})"),
        );
    }
    what_ran_is_what_was_layered(rig);
}

/// The commands a run of `verb` added to what `bs` ran must be commands the build system
/// layers, and the verb's own among them: a test that runs a command other than the one it
/// layers, or a query that runs with a variable of the process, is found here. A command is
/// compared whole, so every verb the kind has is held, not only a build.
fn ran_as_layered(ran: &[Command], allowed: &[Command], own: &Command, what: &str) {
    same(
        ran.iter().all(|c| allowed.contains(c)),
        true,
        &format!("every command of {what} is one the build system layers, ran {ran:?}"),
    );
    same(
        ran.contains(own),
        true,
        &format!("the command {what} ran is the one it layers, ran {ran:?}"),
    );
}

fn what_ran_is_what_was_layered<R: BuildSystemRig>(rig: &R) {
    let mut bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    let allowed: Vec<Command> = verbs_of(bs.kind())
        .into_iter()
        .map(|verb| layered(&bs, &case, verb))
        .collect();
    let before = rig.executed(&bs).len();
    must(
        bs.build(&case.inv(Verb::Build, None), &Beats::counting()),
        "build",
    );
    ran_as_layered(
        &rig.executed(&bs)[before..],
        &allowed,
        &layered(&bs, &case, Verb::Build),
        "a build",
    );
    let before = rig.executed(&bs).len();
    must(test_outcome(&mut bs, &case, &Beats::counting()), "test");
    ran_as_layered(
        &rig.executed(&bs)[before..],
        &allowed,
        &layered(&bs, &case, Verb::Test),
        "a test",
    );
}

fn base_flags_come_before_target_flags_in_order<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    for verb in verbs_of(bs.kind()) {
        let command = layered(&bs, &case, verb);
        // The flags of the authors, in the order they stand in: the base's then the target's,
        // each as often as it was written (a flag both wrote is passed twice, never merged).
        let authored: Vec<&str> = command
            .argv
            .iter()
            .map(String::as_str)
            .filter(|a| AUTHORED.contains(a))
            .collect();
        same(
            authored,
            AUTHORED.to_vec(),
            &format!("the flags the authors wrote in the argument vector of the {verb} verb"),
        );
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
    case.system.flags = vec![
        "--v={release_version}-{build_number}".to_owned(),
        "--p={platform}".to_owned(),
    ];
    case.system.env = BTreeMap::from([
        ("BASE".to_owned(), "{build_number}".to_owned()),
        ("PLAT".to_owned(), "{platform}".to_owned()),
    ]);
    case.target.flags = vec![
        "--c={source_commit}".to_owned(),
        "--r={revision}@{platform}".to_owned(),
        "--t={timestamp_ms}".to_owned(),
        "--lit={{x}}".to_owned(),
    ];
    case.target.env = BTreeMap::from([
        ("STAMP".to_owned(), "{release_version}".to_owned()),
        ("COMMIT".to_owned(), "{source_commit}-{revision}".to_owned()),
        ("TS".to_owned(), "{timestamp_ms}".to_owned()),
        ("ESC".to_owned(), "{{e}}".to_owned()),
    ]);
    for verb in verbs_of(bs.kind()) {
        let command = layered(&bs, &case, verb);
        for flag in [
            "--v=1.2.3-41",
            "--p=linux-x86_64",
            "--c=abc123",
            "--r=0123@linux-x86_64",
            "--t=1700",
            "--lit={x}",
        ] {
            same(
                command.argv.iter().any(|a| a == flag),
                true,
                &format!("{flag} is in the argument vector of the {verb} verb: {command:?}"),
            );
        }
        for (key, value) in [
            ("STAMP", "1.2.3"),
            ("COMMIT", "abc123-0123"),
            ("TS", "1700"),
            ("ESC", "{e}"),
            ("BASE", "41"),
            ("PLAT", "linux-x86_64"),
        ] {
            same(
                env_of(&command, key),
                vec![value],
                &format!("a placeholder in the environment value of {key} of the {verb} verb"),
            );
        }
        // The only brace left is the one a doubled brace wrote.
        same(
            command
                .argv
                .iter()
                .filter(|a| *a != "--lit={x}")
                .any(|a| a.contains('{')),
            false,
            &format!("a placeholder left in the argument vector of the {verb} verb"),
        );
    }
    unknown_placeholders_are_refused(rig);
}

/// A flag or an environment value with a placeholder that does not exist, or a brace that is
/// neither one nor a doubled one, has no layering: nothing is run with it.
fn unknown_placeholders_are_refused<R: BuildSystemRig>(rig: &R) {
    let mut bs = rig.system(&BuildWorld::Normal);
    let kind = bs.kind();
    let refused = |case: &Case| -> Vec<String> {
        verbs_of(kind)
            .into_iter()
            .map(|verb| {
                let report = case.report();
                let report = (verb == Verb::Test).then_some(report.as_str());
                format!("{:?}", bs.command(&case.inv(verb, report)))
            })
            .collect()
    };
    let mut flag = Case::new(kind);
    flag.target.flags = vec!["--x={nope}".to_owned()];
    let mut brace = Case::new(kind);
    brace.system.env = BTreeMap::from([("E".to_owned(), "a}".to_owned())]);
    for case in [&flag, &brace] {
        for answer in refused(case) {
            same(
                answer.starts_with("Err(Permanent("),
                true,
                &format!("the command of a flag or value with a bad brace, got {answer}"),
            );
        }
    }
    let before = rig.executed(&bs).len();
    let failure = must_err(
        bs.build(&flag.inv(Verb::Build, None), &Beats::counting()),
        "a build with a placeholder that does not exist",
    );
    same(
        matches!(failure, SeamError::Permanent(_)),
        true,
        "a build with a placeholder that does not exist is Permanent",
    );
    same(
        rig.executed(&bs).len(),
        before,
        "the commands run for a build that could not be layered",
    );
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
    // The revision and the platform are placeholders as well: no flag names them, so they do
    // not reach the command either.
    "9999".clone_into(&mut case.revision);
    "darwin-arm64".clone_into(&mut case.platform);
    for (verb, before) in verbs.iter().zip(before) {
        same(
            layered(&bs, &case, *verb),
            before,
            &format!(
                "the {verb} command when only the stamp, revision and platform changed and no \
                 flag names them"
            ),
        );
    }
}

fn ucdk_owned_flags_come_after_the_authors<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    for verb in verbs_of(bs.kind()) {
        // Before the first flag of an author: the binary, the startup arguments, the verb word
        // and the config file and mode. After the last: the flags ucdk owns, report path
        // included, and the tail of the verb. Both are the contract's, for the kind.
        same(
            around_authors(&layered(&bs, &case, verb).argv),
            around_authors(&contract(&case, verb).argv),
            &format!("what ucdk owns around the flags of the authors in the {verb} verb"),
        );
    }
    // The flag ucdk owns names a path the engine gives: a test without one has no layering, and
    // another verb with one has none either.
    let no_report = must_err(
        bs.command(&case.inv(Verb::Test, None)),
        "the command of a test without a report path",
    );
    same(
        matches!(no_report, SeamError::Permanent(_)),
        true,
        "a test without a report path is Permanent",
    );
    let stray = must_err(
        bs.command(&case.inv(Verb::Build, Some("/scratch/t1/stray"))),
        "the command of a build given a report path",
    );
    same(
        matches!(stray, SeamError::Permanent(_)),
        true,
        "a build given a report path is Permanent",
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
