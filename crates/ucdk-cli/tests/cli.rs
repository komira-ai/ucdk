// SPDX-License-Identifier: Apache-2.0
//! The command line's own surface: usage errors, the version, and the real binary.

mod common;

use std::process::Command;

use common::{cmd, pair, scratch, write_list, write_text};
use ucdk_cli::{USAGE, World};

#[test]
fn cli_prints_version() {
    let ran = cmd(&mut World::new(), &["--version"]);
    assert_eq!((ran.code, ran.err.as_str()), (0, ""));
    assert_eq!(ran.out, format!("ucdk {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn unknown_flag_exits_2_with_usage() {
    let mut world = World::new();
    let ran = cmd(
        &mut world,
        &["plan", "--provider=fake", "--input=list.pb", "--bogus=1"],
    );
    assert_eq!((ran.code, ran.out.as_str()), (2, ""));
    assert!(
        ran.err
            .starts_with("ucdk: `--bogus` is not a flag of this command\nusage: ucdk --version\n"),
        "{}",
        ran.err
    );
    assert!(ran.err.contains(USAGE));
    assert_eq!(world.fake.calls().len(), 0);
}

#[test]
fn usage_errors_name_the_problem_and_exit_2() {
    let hash = "0".repeat(64);
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec![], "a command is needed"),
        (vec!["--version", "extra"], "`extra` is not a flag"),
        (vec!["list"], "`list` is not a command"),
        (vec!["plan", "list.pb"], "`list.pb` is not a flag"),
        (vec!["plan", "--provider"], "`--provider` needs a value"),
        (
            vec!["plan", "--provider=fake", "--provider=fake"],
            "`--provider` is given twice",
        ),
        (vec!["plan", "--provider=fake"], "`--input` is needed"),
        (
            vec!["plan", "--provider=cloud", "--input=a"],
            "`cloud` is not a provider",
        ),
        (
            vec!["plan", "--provider=fake", "--input=a", "--mode=both"],
            "`both` is not a mode",
        ),
        (
            vec!["apply", "--provider=fake", "--input=a", "--plan=b"],
            "`--expect-plan-hash` is needed",
        ),
        (
            vec![
                "apply",
                "--provider=fake",
                "--input=a",
                "--plan=b",
                "--expect-plan-hash=xyz",
            ],
            "`xyz` is not a plan hash",
        ),
        (
            vec![
                "destroy",
                "--provider=fake",
                "--input=a",
                "--plan=b",
                "--expect-plan-hash",
                &hash,
                "--out=c",
            ],
            "`--out` is not a flag of this command",
        ),
    ];
    for (args, message) in cases {
        let ran = cmd(&mut World::new(), &args);
        assert_eq!((ran.code, ran.out.as_str()), (2, ""), "{args:?}");
        assert!(ran.err.contains(message), "{args:?}: {}", ran.err);
        assert!(ran.err.contains(USAGE), "{args:?}");
    }
}

#[test]
fn files_that_cannot_be_used_are_reported() {
    let dir = scratch("files_that_cannot_be_used_are_reported");
    let list = write_list(&dir, "list.pb", pair());
    let missing = dir.join("missing").to_string_lossy().into_owned();
    let garbage = write_text(&dir, "garbage.pb", "this is not a protobuf message");
    let empty = write_list(&dir, "empty.pb", Vec::new());
    let inside_missing = dir.join("missing/plan.txt").to_string_lossy().into_owned();
    let hash = "0".repeat(64);
    let mut world = World::new();

    let cases: Vec<(Vec<String>, u8, &str)> = vec![
        (
            vec![
                "plan".into(),
                "--provider=fake".into(),
                format!("--input={missing}"),
            ],
            2,
            "cannot read",
        ),
        (
            vec![
                "plan".into(),
                "--provider=fake".into(),
                format!("--input={garbage}"),
            ],
            2,
            "is not a binary resource list",
        ),
        (
            vec![
                "plan".into(),
                "--provider=fake".into(),
                format!("--input={list}"),
                format!("--out={inside_missing}"),
            ],
            2,
            "cannot write",
        ),
        (
            vec![
                "apply".into(),
                "--provider=fake".into(),
                format!("--input={list}"),
                format!("--plan={missing}"),
                format!("--expect-plan-hash={hash}"),
            ],
            2,
            "cannot read",
        ),
        (
            vec![
                "plan".into(),
                "--provider=fake".into(),
                "--mode=destroy".into(),
                format!("--input={empty}"),
            ],
            3,
            "the file names nothing to destroy",
        ),
    ];
    for (args, code, message) in cases {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let ran = cmd(&mut world, &args);
        assert_eq!(ran.code, code, "{args:?}: {ran:?}");
        assert!(ran.err.contains(message), "{args:?}: {}", ran.err);
    }
    assert_eq!(common::mutations(&world.fake), 0);
}

#[test]
fn a_list_with_findings_is_refused_with_every_finding() {
    let dir = scratch("a_list_with_findings_is_refused_with_every_finding");
    let list = write_list(
        &dir,
        "list.pb",
        vec![
            common::service_of("example-a", false, &["example-missing"]),
            common::service_of("example-b", false, &["example-gone"]),
        ],
    );
    let ran = cmd(
        &mut World::new(),
        &["plan", "--provider=fake", &format!("--input={list}")],
    );
    assert_eq!(ran.code, 3, "{ran:?}");
    assert!(ran.err.contains("example-missing"), "{}", ran.err);
    assert!(ran.err.contains("example-gone"), "{}", ran.err);
}

#[test]
fn a_flag_may_take_its_value_as_the_next_argument() {
    let dir = scratch("a_flag_may_take_its_value_as_the_next_argument");
    let list = write_list(&dir, "list.pb", pair());
    let ran = cmd(
        &mut World::new(),
        &["plan", "--provider", "fake", "--input", &list],
    );
    assert_eq!(ran.code, 0, "{ran:?}");
    assert!(
        ran.out
            .starts_with("ucdk plan\nprovider: fake\nmode: apply\nplan-hash: ")
    );
}

#[test]
fn binary_unknown_flag_exits_2() {
    let out = Command::new(env!("CARGO_BIN_EXE_ucdk"))
        .args(["plan", "--provider=fake", "--input=list.pb", "--bogus=1"])
        .output()
        .expect("run ucdk");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("`--bogus` is not a flag"));
}

#[test]
fn binary_version_exits_0() {
    let out = Command::new(env!("CARGO_BIN_EXE_ucdk"))
        .arg("--version")
        .output()
        .expect("run ucdk");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("ucdk {}\n", env!("CARGO_PKG_VERSION"))
    );
    assert!(out.stderr.is_empty());
}
