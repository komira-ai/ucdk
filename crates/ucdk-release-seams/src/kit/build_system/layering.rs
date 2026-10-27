// SPDX-License-Identifier: Apache-2.0
//! The scenarios over the command a build system layers: the flags and the environment an author
//! wrote, the placeholders in them, and what ucdk owns around them.

use std::collections::BTreeMap;

use ucdk_release_proto::v1::{BuildStamp, BuildSystemKind};

use super::{BuildSystemRig, BuildWorld, Case, layered};
use crate::kit::{Beats, must_err, must_some, same};
use crate::{BuildSystem, Command, SeamError, Verb};

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

/// Every verb a build system of `kind` has: Bazel locates an output with two more.
fn verbs_of(kind: BuildSystemKind) -> Vec<Verb> {
    if kind == BuildSystemKind::Bazel {
        vec![Verb::Build, Verb::Test, Verb::Query, Verb::Info]
    } else {
        vec![Verb::Build, Verb::Test]
    }
}

/// The layering the contract gives `case` and `verb`: the function every build system's
/// `command` stands for, which a scenario compares the part it holds against.
fn contract(case: &Case, verb: Verb) -> Command {
    let report = case.report();
    let report = (verb == Verb::Test).then_some(report.as_str());
    must_some(
        ucdk_release::invocation::command(&case.inv(verb, report)).ok(),
        &format!("the contract's command of the {verb} verb"),
    )
}

/// The positions of the first and the last flag an author wrote.
fn authored_bounds(argv: &[String]) -> (usize, usize) {
    let is_authored = |a: &String| AUTHORED.contains(&a.as_str());
    let missing = format!("no flag of an author in the argument vector {argv:?}");
    (
        must_some(argv.iter().position(is_authored), &missing),
        must_some(argv.iter().rposition(is_authored), &missing),
    )
}

/// What comes before the first flag an author wrote and what comes after the last: the binary,
/// the startup arguments, the verb word and the mode, and the flags ucdk owns with the tail of
/// the verb.
fn around_authors(argv: &[String]) -> (Vec<String>, Vec<String>) {
    let (first, last) = authored_bounds(argv);
    (argv[..first].to_vec(), argv[last + 1..].to_vec())
}

/// Everything from the first flag an author wrote to the last, both included.
fn authored_span(argv: &[String]) -> Vec<String> {
    let (first, last) = authored_bounds(argv);
    argv[first..=last].to_vec()
}

fn env_of<'a>(command: &'a Command, key: &str) -> Vec<&'a str> {
    command
        .env
        .iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .collect()
}

pub(super) fn base_flags_come_before_target_flags_in_order<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    for verb in verbs_of(bs.kind()) {
        // From the first flag of an author to the last stands what they wrote, the base's then
        // the target's, each as often as it was written (a flag both wrote is passed twice,
        // never merged), and nothing else between them. The contract's layering is the oracle,
        // and it is held to what the authors wrote.
        let span = authored_span(&contract(&case, verb).argv);
        same(
            span.iter().map(String::as_str).collect::<Vec<_>>(),
            AUTHORED.to_vec(),
            &format!("the contract's flags of the authors in the {verb} verb"),
        );
        same(
            authored_span(&layered(&bs, &case, verb).argv),
            span,
            &format!("everything from the first flag of an author to the last in the {verb} verb"),
        );
    }
}

pub(super) fn a_target_environment_value_replaces_the_base_value_of_the_same_key<
    R: BuildSystemRig,
>(
    rig: &R,
) {
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

pub(super) fn nothing_is_inherited_from_the_process_environment<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let case = Case::new(bs.kind());
    for verb in verbs_of(bs.kind()) {
        let command = layered(&bs, &case, verb);
        // The order of the pairs is not held here, only which keys there are.
        let mut keys: Vec<&str> = command.env.iter().map(|(k, _)| k.as_str()).collect();
        keys.sort_unstable();
        same(
            keys,
            vec!["KEEP", "X"],
            &format!("the keys of the {verb} verb's environment"),
        );
    }
}

pub(super) fn placeholders_are_substituted_in_flag_and_environment_values<R: BuildSystemRig>(
    rig: &R,
) {
    let bs = rig.system(&BuildWorld::Normal);
    let mut case = Case::new(bs.kind());
    // Every placeholder in each of the four places an author writes one: a base flag, a target
    // flag, a base environment value and a target environment value.
    const ALL: &str =
        "{release_version}|{build_number}|{source_commit}|{revision}|{timestamp_ms}|{platform}";
    case.system.flags = vec![
        "--v={release_version}-{build_number}".to_owned(),
        "--p={platform}".to_owned(),
        format!("--ball={ALL}"),
    ];
    case.system.env = BTreeMap::from([
        ("BASE".to_owned(), "{build_number}".to_owned()),
        ("PLAT".to_owned(), "{platform}".to_owned()),
        ("BALL".to_owned(), ALL.to_owned()),
    ]);
    case.target.flags = vec![
        "--c={source_commit}".to_owned(),
        "--r={revision}@{platform}".to_owned(),
        "--t={timestamp_ms}".to_owned(),
        "--lit={{x}}".to_owned(),
        format!("--tall={ALL}"),
    ];
    case.target.env = BTreeMap::from([
        ("STAMP".to_owned(), "{release_version}".to_owned()),
        ("COMMIT".to_owned(), "{source_commit}-{revision}".to_owned()),
        ("TS".to_owned(), "{timestamp_ms}".to_owned()),
        ("ESC".to_owned(), "{{e}}".to_owned()),
        ("TALL".to_owned(), ALL.to_owned()),
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
            "--ball=1.2.3|41|abc123|0123|1700|linux-x86_64",
            "--tall=1.2.3|41|abc123|0123|1700|linux-x86_64",
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
            ("BALL", "1.2.3|41|abc123|0123|1700|linux-x86_64"),
            ("TALL", "1.2.3|41|abc123|0123|1700|linux-x86_64"),
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
    names_are_not_substituted(rig);
    unknown_placeholders_are_refused(rig);
}

/// The binary, the config file, the mode and the label are written as they are, whatever they
/// hold: a placeholder in one is not replaced, and a brace in one is not refused.
fn names_are_not_substituted<R: BuildSystemRig>(rig: &R) {
    let bs = rig.system(&BuildWorld::Normal);
    let mut case = Case::new(bs.kind());
    case.system.binary = "/tools/{platform}/build}".to_owned();
    case.system.config_file = "/cfg/{platform}/}.rc".to_owned();
    case.system.mode = "{platform}}".to_owned();
    case.target.label = "//{platform}:out}".to_owned();
    let pieces = [
        &case.system.binary,
        &case.system.config_file,
        &case.system.mode,
        &case.target.label,
    ];
    for verb in verbs_of(bs.kind()) {
        let command = layered(&bs, &case, verb);
        // The label is held in the verb that is a build; the others are held to what they own
        // by the scenario of the flags ucdk owns.
        let named = if verb == Verb::Build { 4 } else { 3 };
        for piece in &pieces[..named] {
            same(
                command.argv.iter().any(|a| a.contains(piece.as_str())),
                true,
                &format!(
                    "{piece} as written in the argument vector of the {verb} verb: {command:?}"
                ),
            );
        }
    }
}

/// A flag or an environment value with a placeholder that does not exist, or a brace that is
/// neither one nor a doubled one, has no layering: nothing is run with it.
fn unknown_placeholders_are_refused<R: BuildSystemRig>(rig: &R) {
    let mut bs = rig.system(&BuildWorld::Normal);
    let kind = bs.kind();
    let refused = |case: &Case| -> Vec<Result<Command, SeamError>> {
        verbs_of(kind)
            .into_iter()
            .map(|verb| {
                let report = case.report();
                let report = (verb == Verb::Test).then_some(report.as_str());
                bs.command(&case.inv(verb, report))
            })
            .collect()
    };
    let mut flag = Case::new(kind);
    flag.target.flags = vec!["--x={nope}".to_owned()];
    let mut brace = Case::new(kind);
    brace.system.env = BTreeMap::from([("E".to_owned(), "a}".to_owned())]);
    // The same in each of the other places an author writes a flag or a value.
    let mut base_flag = Case::new(kind);
    base_flag.system.flags = vec!["--x={nope}".to_owned()];
    let mut target_env = Case::new(kind);
    target_env.target.env = BTreeMap::from([("E".to_owned(), "{nope}".to_owned())]);
    let mut base_env = Case::new(kind);
    base_env.system.env = BTreeMap::from([("E".to_owned(), "{nope}".to_owned())]);
    let mut target_brace = Case::new(kind);
    target_brace.target.flags = vec!["--x=a}".to_owned()];
    for case in [
        &flag,
        &brace,
        &base_flag,
        &target_env,
        &base_env,
        &target_brace,
    ] {
        for answer in refused(case) {
            same(
                matches!(answer, Err(SeamError::Permanent(_))),
                true,
                &format!("the command of a flag or value with a bad brace, got {answer:?}"),
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

pub(super) fn the_stamp_reaches_a_build_only_through_placeholders<R: BuildSystemRig>(rig: &R) {
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

pub(super) fn ucdk_owned_flags_come_after_the_authors<R: BuildSystemRig>(rig: &R) {
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
