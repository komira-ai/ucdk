// SPDX-License-Identifier: Apache-2.0
//! The `check` gates and the verbs, run against a fake runner and a small project on disk.

mod common;

use std::path::PathBuf;
use std::process::Command;

use common::{Fake, cache_tools, scratch, strings, write};
use xtask::check::{render, run};
use xtask::cli;
use xtask::event::Event;

fn project(name: &str) -> PathBuf {
    let root = scratch(name);
    write(
        &root,
        "crates/c/src/lib.rs",
        "// SPDX-License-Identifier: Apache-2.0\n",
    );
    write(
        &root,
        "xtask/src/lib.rs",
        "// SPDX-License-Identifier: Apache-2.0\n",
    );
    write(
        &root,
        ".github/workflows/ci.yml",
        "jobs:\n  check:\n    steps:\n      - run: cargo xtask check\n",
    );
    cache_tools(&root);
    root
}

fn names(gates: &[xtask::check::Gate]) -> Vec<&str> {
    gates.iter().map(|(name, _, _)| *name).collect()
}

fn failed(gates: &[xtask::check::Gate]) -> Vec<&str> {
    gates
        .iter()
        .filter(|(_, f, _)| !f.is_empty())
        .map(|(name, _, _)| *name)
        .collect()
}

const STATIC_GATES: [&str; 12] = [
    "tools",
    "tree",
    "fmt",
    "clippy",
    "deny",
    "spdx",
    "run-lines",
    "main-rs",
    "purity",
    "direction",
    "public-lint tree",
    "public-lint pull request text",
];

#[test]
fn check_runs_the_static_gates_with_the_pinned_tools_on_the_path() {
    let root = project("check-gates");
    let fake = Fake::default();
    let gates = run(&root, &fake, &Event::default(), None);
    assert_eq!(names(&gates), STATIC_GATES);
    assert!(failed(&gates).is_empty(), "{gates:?}");
    let log = fake.log.borrow().clone();
    assert_eq!(log[0], "cargo fmt --all --check");
    assert_eq!(
        log[1],
        "cargo clippy --workspace --all-targets --locked -- -D warnings"
    );
    assert_eq!(
        log[2],
        "cargo deny --log-level warn --manifest-path Cargo.toml --all-features --config deny.toml check licenses bans sources"
    );
    assert_eq!(log.len(), 3, "no tests run in this job: {log:?}");
}

#[test]
fn check_runs_every_gate_even_after_one_fails() {
    let root = project("check-keeps-going");
    write(&root, "crates/c/src/bad.rs", "fn no_header() {}\n");
    let fake = Fake {
        fail_on: vec!["clippy"],
        ..Fake::default()
    };
    let gates = run(&root, &fake, &Event::default(), None);
    assert_eq!(names(&gates), STATIC_GATES);
    assert_eq!(failed(&gates), ["clippy", "spdx"]);
    let (code, text) = render(&gates);
    assert_eq!(code, 1);
    assert!(
        text.contains("== clippy: FAILED (all workspace targets)\n[command] cargo clippy"),
        "{text}"
    );
    assert!(
        text.contains("== fmt: ok (all workspace crates)\n"),
        "{text}"
    );
    assert!(
        text.contains("[spdx-missing] crates/c/src/bad.rs"),
        "{text}"
    );
    assert!(
        text.ends_with("== xtask check: 2 of 12 gates failed\n"),
        "{text}"
    );
    assert_eq!(
        render(&[]),
        (0, "== xtask check: 0 of 0 gates failed\n".to_owned())
    );
}

#[test]
fn check_reports_an_unreadable_tree() {
    let gates = run(
        &scratch("check-missing").join("nope"),
        &Fake::default(),
        &Event::default(),
        None,
    );
    assert!(failed(&gates).contains(&"tree"), "{:?}", failed(&gates));
}

fn pull_request(base: &str) -> Event {
    Event {
        is_pull_request: true,
        base_sha: base.to_owned(),
        ..Event::default()
    }
}

fn log_line(email: &str, message: &str) -> String {
    format!("abcdef0123456789\u{1f}0123abcd\u{1f}{email}\u{1f}{message}\0")
}

#[test]
fn a_pull_request_adds_the_sign_off_and_commit_message_gates() {
    let root = project("check-pr");
    let unsigned = Fake {
        answers: vec![("git log", log_line("a@example.org", "Add x\n"))],
        ..Fake::default()
    };
    let gates = run(&root, &unsigned, &pull_request("base1"), None);
    assert_eq!(names(&gates)[12..], ["dco", "commit-message lint"]);
    assert_eq!(failed(&gates), ["dco"]);
    assert!(
        unsigned
            .log
            .borrow()
            .iter()
            .any(|c| c.ends_with("base1..HEAD"))
    );

    let signed = log_line(
        "a@example.org",
        "Add x\n\nSigned-off-by: A <a@example.org>\n",
    );
    let ok = Fake {
        answers: vec![("git log", signed)],
        ..Fake::default()
    };
    assert!(failed(&run(&root, &ok, &pull_request("base1"), None)).is_empty());
    // The flag replaces the event's base.
    let flagged = Fake::default();
    run(&root, &flagged, &pull_request("base1"), Some("flagbase"));
    assert!(
        flagged
            .log
            .borrow()
            .iter()
            .any(|c| c.ends_with("flagbase..HEAD"))
    );
}

#[test]
fn a_pull_request_without_a_base_or_with_an_unreadable_range_fails() {
    let root = project("check-pr-errors");
    let gates = run(&root, &Fake::default(), &pull_request(""), None);
    assert_eq!(failed(&gates), ["dco"]);
    assert_eq!(gates[12].1[0].rule, "dco-no-base");

    let broken = Fake {
        fail_on: vec!["git log"],
        ..Fake::default()
    };
    let gates = run(&root, &broken, &pull_request("base1"), None);
    assert_eq!(failed(&gates), ["dco"]);
    assert_eq!(gates[12].1[0].rule, "git");
}

#[test]
fn the_pull_request_text_is_linted() {
    let word = ["buc", "ket"].concat();
    let event = Event {
        title: word,
        ..Event::default()
    };
    let gates = run(&project("check-text"), &Fake::default(), &event, None);
    assert_eq!(failed(&gates), ["public-lint pull request text"]);
}

fn env_of(pairs: Vec<(&'static str, String)>) -> impl Fn(&str) -> Option<String> {
    move |name| {
        pairs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.clone())
    }
}

#[test]
fn the_check_verb_reads_the_event_file_named_by_the_platform_variable() {
    let root = project("verb-check");
    let event = root.join("event.json");
    std::fs::write(
        &event,
        r#"{"pull_request":{"base":{"sha":"b1"},"title":"Add x","body":"Body"}}"#,
    )
    .expect("event");
    let env = env_of(vec![("GITHUB_EVENT_PATH", event.display().to_string())]);
    let fake = Fake {
        answers: vec![(
            "git log",
            log_line(
                "a@example.org",
                "Add x\n\nSigned-off-by: A <a@example.org>\n",
            ),
        )],
        ..Fake::default()
    };
    let (code, text) = cli::run(&strings(&["check"]), &env, &fake, &root);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("== dco: ok (1 commit checked)"), "{text}");

    let none = env_of(vec![]);
    let (code, text) = cli::run(
        &strings(&["check", "--base", "b1"]),
        &none,
        &Fake::default(),
        &root,
    );
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("== dco: ok (0 commits checked)"), "{text}");

    let (code, text) = cli::run(
        &strings(&["check", "--base"]),
        &none,
        &Fake::default(),
        &root,
    );
    assert_eq!(code, 0, "{text}");
}

#[test]
fn the_check_verb_reports_an_unreadable_or_invalid_event_file() {
    let root = project("verb-check-bad-event");
    let missing = env_of(vec![(
        "GITHUB_EVENT_PATH",
        root.join("nope.json").display().to_string(),
    )]);
    let (code, text) = cli::run(&strings(&["check"]), &missing, &Fake::default(), &root);
    assert_eq!((code, text.contains("No such file")), (1, true), "{text}");
    write(&root, "bad.json", "{");
    let invalid = env_of(vec![(
        "GITHUB_EVENT_PATH",
        root.join("bad.json").display().to_string(),
    )]);
    let (code, text) = cli::run(&strings(&["check"]), &invalid, &Fake::default(), &root);
    assert_eq!((code, text.starts_with("event file:")), (1, true), "{text}");
}

#[test]
fn the_fetch_and_coverage_verbs_report_their_findings() {
    let none = env_of(vec![]);
    let root = project("verb-fetch");
    assert_eq!(
        cli::run(&strings(&["fetch"]), &none, &Fake::default(), &root),
        (0, String::new())
    );
    let broken = Fake {
        fail_on: vec!["curl"],
        ..Fake::default()
    };
    let (code, text) = cli::run(
        &strings(&["fetch"]),
        &none,
        &broken,
        &scratch("verb-fetch-fail"),
    );
    assert_eq!(code, 1);
    assert!(text.starts_with("[fetch] cargo-deny:"), "{text}");

    let (code, text) = cli::run(&strings(&["coverage"]), &none, &Fake::default(), &root);
    assert_eq!(code, 1);
    assert!(text.contains("[coverage-read]"), "{text}");
    let (code, text) = cli::run(
        &strings(&["coverage"]),
        &none,
        &Fake::default(),
        &scratch("verb-cov-ok"),
    );
    assert_eq!((code, text.contains("[fetch]")), (1, true), "{text}");
}

#[test]
fn the_coverage_verb_prints_the_per_crate_table() {
    let root = project("verb-cov-table");
    let hook: common::Hook = Box::new(|cmd| {
        if cmd.args.iter().any(|a| a == "llvm-cov") {
            let base = cmd.dir.canonicalize().expect("root");
            let line = |krate: &str| {
                format!(
                    "SF:{}/{krate}/src/lib.rs\nDA:1,1\nBRDA:1,0,0,1\nend_of_record\n",
                    base.display()
                )
            };
            write(
                &cmd.dir,
                xtask::coverage::LCOV,
                &[line("crates/c"), line("xtask")].concat(),
            );
        }
    });
    let fake = Fake {
        on_run: Some(hook),
        ..Fake::default()
    };
    let (code, text) = cli::run(&strings(&["coverage"]), &env_of(vec![]), &fake, &root);
    assert_eq!(code, 0, "{text}");
    assert!(
        text.contains("c: lines 100.00% (1/1), branches 100.00% (1/1)"),
        "{text}"
    );
}

#[test]
fn an_unknown_verb_prints_usage_and_exits_two() {
    let (code, text) = cli::run(
        &strings(&["frobnicate"]),
        &env_of(vec![]),
        &Fake::default(),
        &scratch("verb-usage"),
    );
    assert_eq!(code, 2);
    assert!(text.starts_with("usage: cargo xtask <verb>"), "{text}");
    assert_eq!(
        cli::run(
            &[],
            &env_of(vec![]),
            &Fake::default(),
            &scratch("verb-none")
        )
        .0,
        2
    );
}

#[test]
fn the_binary_forwards_to_the_library() {
    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .arg("frobnicate")
        .output()
        .expect("run xtask");
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("usage: cargo xtask <verb>"));
}

#[test]
fn the_static_gates_fetch_only_cargo_deny() {
    let root = project("check-fetch-scope");
    std::fs::remove_dir_all(root.join("target")).expect("drop the cached tools");
    let fake = Fake {
        fail_on: vec!["curl"],
        ..Fake::default()
    };
    let gates = run(&root, &fake, &Event::default(), None);
    let tools = gates.iter().find(|g| g.0 == "tools").expect("tools gate");
    assert_eq!(
        tools
            .1
            .iter()
            .map(|f| f.subject.as_str())
            .collect::<Vec<_>>(),
        ["cargo-deny"]
    );
    assert!(
        fake.log
            .borrow()
            .iter()
            .all(|c| !c.contains("cargo-llvm-cov")),
        "{:?}",
        fake.log.borrow()
    );
}

#[test]
fn every_gate_prints_what_it_examined() {
    let root = project("check-examined");
    let fake = Fake {
        answers: vec![(
            "git log",
            log_line(
                "a@example.org",
                "Add x\n\nSigned-off-by: A <a@example.org>\n",
            ) + &log_line(
                "a@example.org",
                "Add y\n\nSigned-off-by: A <a@example.org>\n",
            ),
        )],
        ..Fake::default()
    };
    let (code, text) = render(&run(&root, &fake, &pull_request("base1"), None));
    assert_eq!(code, 0, "{text}");
    for line in text
        .lines()
        .filter(|l| l.starts_with("== ") && !l.contains("gates failed"))
    {
        assert!(line.ends_with(')'), "{line}");
    }
    assert!(text.contains("== dco: ok (2 commits checked)\n"), "{text}");
    assert!(
        text.contains("== commit-message lint: ok (2 commits checked)\n"),
        "{text}"
    );
    assert!(text.contains("== spdx: ok ("), "{text}");
    assert!(
        text.contains("== run-lines: ok (1 workflow file)\n"),
        "{text}"
    );
}
