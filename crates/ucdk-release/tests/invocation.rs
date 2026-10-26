// SPDX-License-Identifier: Apache-2.0
//! The layering of one build-system invocation: the argument vector of each kind and verb, the
//! order of the author's flags and ucdk's own, the environment and the placeholders.

mod common;

use common::{PLATFORM, REVISION, pairs, stamp, strings};
use ucdk_release::flags::{BraceError, reserved};
use ucdk_release::invocation::{
    Command, Invocation, InvocationError, Verb, command, report_file_name,
};
use ucdk_release_proto::v1::BuildSystemKind::{Bazel, Buck2, Unspecified};
use ucdk_release_proto::v1::{BuildStamp, BuildSystem, BuildSystemKind, Target};

const LABEL: &str = "//release:tool";
const BUCK2_REPORT: &str = "/scratch/1/events.json-lines";
const BAZEL_REPORT: &str = "/scratch/1/events.json";

fn system(kind: BuildSystemKind) -> BuildSystem {
    BuildSystem {
        kind: kind as i32,
        binary: kind_binary(kind).to_owned(),
        config_file: format!("release/{}rc", kind_binary(kind)),
        mode: "release".to_owned(),
        flags: strings(&["--base-one", "--base-two={platform}"]),
        env: pairs(&[("PATH", "/usr/bin")]),
    }
}

fn kind_binary(kind: BuildSystemKind) -> &'static str {
    match kind {
        Buck2 => "buck2",
        Bazel => "bazel",
        Unspecified => "",
    }
}

fn target() -> Target {
    Target {
        label: LABEL.to_owned(),
        flags: strings(&["--target-one={release_version}", "--target-two"]),
        env: pairs(&[("TOOL_MODE", "{build_number}")]),
    }
}

fn run(
    system: &BuildSystem,
    target: &Target,
    verb: Verb,
    report: Option<&str>,
) -> Result<Command, InvocationError> {
    command(&Invocation {
        system,
        target,
        verb,
        report,
        stamp: &stamp(),
        revision: REVISION,
        platform: PLATFORM,
    })
}

fn argv(kind: BuildSystemKind, verb: Verb, report: Option<&str>) -> Vec<String> {
    run(&system(kind), &target(), verb, report)
        .expect("layers")
        .argv
}

#[test]
fn argv_goldens() {
    // One golden argument vector per kind and verb, each over a full configuration: a config
    // file, a mode, two base flags, two target flags, a placeholder in each list.
    let flags = [
        "--base-one",
        "--base-two=linux-x86_64",
        "--target-one=1.2.3",
        "--target-two",
    ];
    let golden = |head: &[&str], owned: &[&str], tail: &[&str]| -> Vec<String> {
        [head, &flags, owned, tail]
            .concat()
            .into_iter()
            .map(str::to_owned)
            .collect()
    };
    let buck2_head = |verb| {
        [
            "buck2",
            verb,
            "--config-file",
            "release/buck2rc",
            "@release",
        ]
    };
    let bazel_head = |verb| {
        [
            "bazel",
            "--bazelrc=release/bazelrc",
            verb,
            "--config=release",
        ]
    };

    assert_eq!(
        argv(Buck2, Verb::Build, None),
        golden(&buck2_head("build"), &["--show-full-output"], &[LABEL])
    );
    assert_eq!(
        argv(Buck2, Verb::Test, Some(BUCK2_REPORT)),
        golden(
            &buck2_head("test"),
            &["--event-log=/scratch/1/events.json-lines"],
            &[LABEL]
        )
    );
    assert_eq!(
        argv(Bazel, Verb::Build, None),
        golden(&bazel_head("build"), &[], &["--", LABEL])
    );
    assert_eq!(
        argv(Bazel, Verb::Query, None),
        golden(&bazel_head("cquery"), &["--output=files"], &["--", LABEL])
    );
    assert_eq!(
        argv(Bazel, Verb::Info, None),
        golden(&bazel_head("info"), &[], &["execution_root"])
    );
    assert_eq!(
        argv(Bazel, Verb::Test, Some(BAZEL_REPORT)),
        golden(
            &bazel_head("test"),
            &["--build_event_json_file=/scratch/1/events.json"],
            &["--", LABEL]
        )
    );
}

#[test]
fn the_report_flag_is_owned_and_the_report_path_is_not_hashed() {
    for (kind, report, flag) in [
        (Buck2, BUCK2_REPORT, "--event-log"),
        (Bazel, BAZEL_REPORT, "--build_event_json_file"),
    ] {
        // The flag that names the report is one the author may not write, so the machine
        // cannot carry it and the path is the engine's alone.
        assert!(reserved(kind, flag).is_some(), "{kind:?}");
        assert!(reserved(kind, &format!("{flag}={report}")).is_some());

        let one = argv(kind, Verb::Test, Some(report));
        let other = argv(kind, Verb::Test, Some("/elsewhere/2/report"));
        let owned = format!("{flag}={report}");
        let at = one.iter().position(|e| *e == owned).expect("owned flag");
        assert_eq!(other[at], format!("{flag}=/elsewhere/2/report"));
        // The path is in that one element and nowhere else, and the environment is the same
        // whatever the path: it is no input of the layering besides that element.
        let mut rest = one.clone();
        rest.remove(at);
        let mut rest_other = other.clone();
        rest_other.remove(at);
        assert_eq!(rest, rest_other);
        assert!(rest.iter().all(|e| !e.contains(report)));
        let env = |path| {
            run(&system(kind), &target(), Verb::Test, Some(path))
                .expect("layers")
                .env
        };
        assert_eq!(env(report), env("/elsewhere/2/report"));
    }
}

#[test]
fn the_report_file_name_is_fixed_per_kind_with_the_suffix_buck2_reads_its_format_from() {
    let buck2 = report_file_name(Buck2).expect("a name");
    assert_eq!(buck2, "events.json-lines");
    // Buck2 picks the encoding and the compression from the suffix, and `.json-lines` is the
    // uncompressed JSON lines one.
    assert!(buck2.ends_with(".json-lines"));
    assert!(![".pb", ".gz", ".zst"].iter().any(|s| buck2.ends_with(s)));
    assert_eq!(report_file_name(Bazel), Some("events.json"));
    assert_eq!(report_file_name(Unspecified), None);
}

#[test]
fn query_and_info_take_the_base_and_target_flags_of_their_build() {
    let build = argv(Bazel, Verb::Build, None);
    let query = argv(Bazel, Verb::Query, None);
    let info = argv(Bazel, Verb::Info, None);
    // Everything up to the verb word and the config argument is the same, and so are the base
    // and the target flags that follow it.
    let flags = &build[4..8];
    assert_eq!(
        flags,
        [
            "--base-one",
            "--base-two=linux-x86_64",
            "--target-one=1.2.3",
            "--target-two"
        ]
    );
    for other in [&query, &info] {
        assert_eq!(other[0..2], build[0..2]);
        assert_eq!(other[3], build[3]);
        assert_eq!(&other[4..8], flags);
    }
    // `info` has no label and no `--`; the verbs that name a target end in both.
    assert_eq!(info.last().map(String::as_str), Some("execution_root"));
    assert!(!info.contains(&LABEL.to_owned()) && !info.contains(&"--".to_owned()));
    assert_eq!(query[query.len() - 2..], ["--", LABEL]);
    // The environment is the build's too.
    let env = |verb| {
        run(&system(Bazel), &target(), verb, None)
            .expect("layers")
            .env
    };
    assert_eq!(env(Verb::Query), env(Verb::Build));
    assert_eq!(env(Verb::Info), env(Verb::Build));
}

#[test]
fn an_empty_config_file_and_mode_add_no_argument() {
    for (kind, verb) in [(Buck2, "build"), (Bazel, "build")] {
        let mut bare = system(kind);
        bare.config_file = String::new();
        bare.mode = String::new();
        bare.flags = Vec::new();
        let bare_target = Target {
            flags: Vec::new(),
            ..target()
        };
        let got = run(&bare, &bare_target, Verb::Build, None).expect("layers");
        let tail: &[&str] = if kind == Bazel {
            &["--", LABEL]
        } else {
            &["--show-full-output", LABEL]
        };
        let want: Vec<String> = [&[kind_binary(kind), verb][..], tail]
            .concat()
            .into_iter()
            .map(str::to_owned)
            .collect();
        assert_eq!(got.argv, want, "{kind:?}");
    }
    // Each is independent: a mode alone and a config file alone.
    let mut mode_only = system(Buck2);
    mode_only.config_file = String::new();
    mode_only.flags = Vec::new();
    let got = run(
        &mode_only,
        &Target {
            flags: Vec::new(),
            ..target()
        },
        Verb::Build,
        None,
    )
    .expect("layers");
    assert_eq!(got.argv[..3], ["buck2", "build", "@release"]);
    let mut file_only = system(Bazel);
    file_only.mode = String::new();
    file_only.flags = Vec::new();
    let got = run(
        &file_only,
        &Target {
            flags: Vec::new(),
            ..target()
        },
        Verb::Build,
        None,
    )
    .expect("layers");
    assert_eq!(
        got.argv[..3],
        ["bazel", "--bazelrc=release/bazelrc", "build"]
    );
    assert_eq!(got.argv[3], "--");
}

#[test]
fn base_flags_precede_target_flags_in_order() {
    for kind in [Buck2, Bazel] {
        let mut sys = system(kind);
        sys.flags = strings(&["-c", "a=1", "--rep=base1", "--rep=base2"]);
        let tgt = Target {
            flags: strings(&["-c", "a=2", "--rep=target"]),
            ..target()
        };
        let got = run(&sys, &tgt, Verb::Build, None).expect("layers").argv;
        let at = got.iter().position(|e| e == "-c").expect("the base flags");
        // Appended, never merged: `a=1` stays beside `a=2`, base values first, so a last-wins
        // flag is decided by the target's occurrence and a repeatable one accumulates.
        assert_eq!(
            got[at..at + 7],
            [
                "-c",
                "a=1",
                "--rep=base1",
                "--rep=base2",
                "-c",
                "a=2",
                "--rep=target"
            ],
            "{kind:?}"
        );
    }
}

#[test]
fn a_target_environment_value_replaces_the_base_value_of_the_same_key() {
    let mut sys = system(Buck2);
    sys.env = pairs(&[
        ("A", "base"),
        ("B", "base"),
        ("PATH", "/usr/bin"),
        ("E", "kept"),
    ]);
    let tgt = Target {
        env: pairs(&[("B", "target"), ("C", "target"), ("E", "")]),
        ..target()
    };
    let got = run(&sys, &tgt, Verb::Build, None).expect("layers").env;
    // Sorted by key; a key in both takes the target's value, never a concatenation, and an
    // empty value is a variable set to the empty text.
    let want: Vec<(String, String)> = [
        ("A", "base"),
        ("B", "target"),
        ("C", "target"),
        ("E", ""),
        ("PATH", "/usr/bin"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    assert_eq!(got, want);
}

#[test]
fn nothing_is_inherited() {
    // The environment is the layered base and target environment and nothing else: a machine
    // that names no variable gives an empty environment, whatever this process has.
    let mut sys = system(Buck2);
    sys.env.clear();
    let tgt = Target {
        env: pairs(&[]),
        ..target()
    };
    assert_eq!(run(&sys, &tgt, Verb::Build, None).expect("layers").env, []);
    // And one that names two has those two, in key order, and no third.
    sys.env = pairs(&[("HOME", "/h"), ("PATH", "/bin")]);
    assert_eq!(
        run(&sys, &tgt, Verb::Build, None).expect("layers").env,
        [
            ("HOME".to_owned(), "/h".to_owned()),
            ("PATH".to_owned(), "/bin".to_owned())
        ]
    );
}

#[test]
fn placeholders_are_substituted_and_doubled_braces_are_literal() {
    let sys = BuildSystem {
        flags: strings(&[
            "--a={release_version}/{build_number}/{source_commit}",
            "--b={revision}/{timestamp_ms}/{platform}",
        ]),
        env: pairs(&[
            ("V", "{release_version}"),
            ("L", "{{literal}} and {{{platform}}}"),
        ]),
        ..system(Buck2)
    };
    let tgt = Target {
        flags: strings(&["--c={{release_version}}", "--d={platform}{platform}"]),
        env: pairs(&[("T", "{timestamp_ms}")]),
        // The label is never substituted.
        label: "//x:{platform}".to_owned(),
    };
    let got = run(&sys, &tgt, Verb::Build, None).expect("layers");
    let flags = &got.argv[5..9];
    assert_eq!(
        flags,
        [
            "--a=1.2.3/41/abc123".to_owned(),
            format!("--b={REVISION}/1700000000000/{PLATFORM}"),
            "--c={release_version}".to_owned(),
            format!("--d={PLATFORM}{PLATFORM}"),
        ]
    );
    assert_eq!(got.argv.last().map(String::as_str), Some("//x:{platform}"));
    assert_eq!(
        got.env,
        [
            ("L".to_owned(), format!("{{literal}} and {{{PLATFORM}}}")),
            ("T".to_owned(), "1700000000000".to_owned()),
            ("V".to_owned(), "1.2.3".to_owned()),
        ]
    );
    // The binary, the config file and the mode are not substituted either.
    let sys = BuildSystem {
        binary: "{platform}/buck2".to_owned(),
        config_file: "{revision}".to_owned(),
        mode: "{platform}".to_owned(),
        flags: Vec::new(),
        ..system(Buck2)
    };
    let got = run(&sys, &target(), Verb::Build, None)
        .expect("layers")
        .argv;
    assert_eq!(
        got[..5],
        [
            "{platform}/buck2",
            "build",
            "--config-file",
            "{revision}",
            "@{platform}"
        ]
    );
}

#[test]
fn an_empty_platform_substitutes_to_nothing() {
    let sys = BuildSystem {
        flags: strings(&["--p=[{platform}]"]),
        ..system(Buck2)
    };
    let got = command(&Invocation {
        system: &sys,
        target: &target(),
        verb: Verb::Build,
        report: None,
        stamp: &BuildStamp::default(),
        revision: "",
        platform: "",
    })
    .expect("layers");
    assert!(got.argv.contains(&"--p=[]".to_owned()));
}

#[test]
fn ucdk_owned_flags_come_after_the_authors() {
    for (kind, verb, report, owned) in [
        (Buck2, Verb::Build, None, "--show-full-output"),
        (Buck2, Verb::Test, Some(BUCK2_REPORT), "--event-log="),
        (Bazel, Verb::Query, None, "--output=files"),
        (
            Bazel,
            Verb::Test,
            Some(BAZEL_REPORT),
            "--build_event_json_file=",
        ),
    ] {
        let got = argv(kind, verb, report);
        let author = got
            .iter()
            .rposition(|e| e == "--target-two")
            .expect("the last author flag");
        let ours = got
            .iter()
            .position(|e| e.starts_with(owned))
            .expect("the owned flag");
        assert_eq!(ours, author + 1, "{kind:?} {verb}");
        // Nothing but the tail follows: the label, with `--` for Bazel.
        let tail = if kind == Bazel { 2 } else { 1 };
        assert_eq!(got.len(), ours + 1 + tail, "{kind:?} {verb}");
        // The owned flags are the ones the author may not write.
        assert!(reserved(kind, &got[ours]).is_some(), "{}", got[ours]);
    }
}

#[test]
fn a_layering_that_cannot_be_made_is_refused_and_says_why() {
    let buck2 = system(Buck2);
    for verb in [Verb::Query, Verb::Info] {
        let err = run(&buck2, &target(), verb, None).expect_err("Buck2 has no such verb");
        assert_eq!(err, InvocationError::Verb { kind: Buck2, verb });
        assert!(err.to_string().contains("has no"), "{err}");
    }
    let err = run(&buck2, &target(), Verb::Test, None).expect_err("a test needs a report");
    assert_eq!(
        err,
        InvocationError::Report {
            verb: Verb::Test,
            given: false
        }
    );
    assert!(err.to_string().contains("needs the path"), "{err}");
    let err = run(&buck2, &target(), Verb::Build, Some("/r")).expect_err("a build has none");
    assert_eq!(
        err,
        InvocationError::Report {
            verb: Verb::Build,
            given: true
        }
    );
    assert!(err.to_string().contains("writes no report"), "{err}");
    for kind in [Unspecified as i32, 99] {
        let sys = BuildSystem {
            kind,
            ..system(Buck2)
        };
        let err = run(&sys, &target(), Verb::Build, None).expect_err("no kind");
        assert_eq!(err, InvocationError::Kind);
        assert!(err.to_string().contains("no known kind"), "{err}");
    }
    for (verb, text) in [
        (Verb::Build, "build"),
        (Verb::Test, "test"),
        (Verb::Query, "query"),
        (Verb::Info, "info"),
    ] {
        assert_eq!(verb.to_string(), text);
    }
}

#[test]
fn a_brace_that_is_no_placeholder_names_where_it_is() {
    let mut sys = system(Buck2);
    sys.flags = strings(&["--ok", "--bad={nope}"]);
    let err = run(&sys, &target(), Verb::Build, None).expect_err("unknown placeholder");
    assert_eq!(
        err,
        InvocationError::Brace {
            at: "the base flag 1".to_owned(),
            error: BraceError::Unknown {
                at: 6,
                name: "nope".to_owned()
            }
        }
    );
    assert!(err.to_string().starts_with("the base flag 1: "), "{err}");

    let tgt = Target {
        flags: strings(&["--x}"]),
        ..target()
    };
    let err = run(&system(Buck2), &tgt, Verb::Build, None).expect_err("stray brace");
    assert!(
        matches!(&err, InvocationError::Brace { at, error: BraceError::Stray { .. } } if at == "the target's flag 0"),
        "{err:?}"
    );

    let tgt = Target {
        env: pairs(&[("K", "{unclosed")]),
        ..target()
    };
    let err = run(&system(Buck2), &tgt, Verb::Build, None).expect_err("unclosed brace");
    assert!(
        matches!(&err, InvocationError::Brace { at, error: BraceError::Unclosed { .. } } if at == "the environment value of K"),
        "{err:?}"
    );
}
