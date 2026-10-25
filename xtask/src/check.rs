// SPDX-License-Identifier: Apache-2.0
//! `cargo xtask check`: the static gates, every one run even after another fails. Tests and
//! coverage are the other job (`cargo xtask coverage`).

use std::path::Path;

use crate::cmd::{Cmd, Runner};
use crate::event::{Event, commit_base};
use crate::{
    Finding, coverage, dco, direction, fetch, lint, purity, runlines, spdx, tree, unsafe_scope,
};

/// One gate's name, what it found, and what it examined (printed with the result).
pub type Gate = (&'static str, Vec<Finding>, String);

fn command_gate(runner: &dyn Runner, name: &'static str, cmd: &Cmd, what: &str) -> Gate {
    let findings = match runner.run(cmd) {
        Ok(_) => Vec::new(),
        Err(e) => vec![Finding::new("command", cmd.to_string(), e)],
    };
    (name, findings, what.to_owned())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The commit-range gates: sign-off and the public lint over the commit messages.
fn commit_gates(root: &Path, runner: &dyn Runner, event: &Event, flag: Option<&str>) -> Vec<Gate> {
    match commit_base(event, flag) {
        Err(finding) => vec![("dco", vec![finding], "no commit".to_owned())],
        Ok(None) => {
            eprintln!("== dco and commit-message lint: skipped (no pull request, no --base)");
            Vec::new()
        }
        Ok(Some(base)) => match dco::commits_since(root, runner, &base) {
            Err(e) => vec![(
                "dco",
                vec![Finding::new("git", &base, e)],
                "no commit".to_owned(),
            )],
            Ok(commits) => {
                let checked = format!("{} checked", plural(commits.len(), "commit", "commits"));
                vec![
                    ("dco", dco::check_dco(&commits), checked.clone()),
                    (
                        "commit-message lint",
                        lint::check_commits(&commits),
                        checked,
                    ),
                ]
            }
        },
    }
}

/// Runs every gate; `flag` is the `--base` flag.
pub fn run(root: &Path, runner: &dyn Runner, event: &Event, flag: Option<&str>) -> Vec<Gate> {
    let bin = fetch::bin_dir(root);
    let files = tree::read_tree(root);
    let (files, read_error) = match files {
        Ok(files) => (files, Vec::new()),
        Err(e) => (Vec::new(), vec![Finding::new("io", "tree", e)]),
    };
    let deny = Cmd::new(
        "cargo",
        &[
            "deny",
            "--log-level",
            "warn",
            "--manifest-path",
            "Cargo.toml",
            "--all-features",
            "--config",
            "deny.toml",
            "check",
            "licenses",
            "bans",
            "sources",
        ],
        root,
    )
    .with_path(&bin);
    let fmt = Cmd::new("cargo", &["fmt", "--all", "--check"], root);
    let clippy = Cmd::new(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
        root,
    );
    let members = coverage::members(root).unwrap_or_default();
    let count = |keep: fn(&str) -> bool| files.iter().filter(|(p, _)| keep(p)).count();
    let file_count = plural(files.len(), "file", "files");
    let mut gates: Vec<Gate> = vec![
        (
            "tools",
            fetch::ensure_only(root, runner, &[fetch::DENY.to_owned()]),
            format!("{} verified", fetch::DENY),
        ),
        ("tree", read_error, format!("{file_count} read")),
        command_gate(runner, "fmt", &fmt, "all workspace crates"),
        command_gate(runner, "clippy", &clippy, "all workspace targets"),
        command_gate(runner, "deny", &deny, "licenses, bans and sources"),
        (
            "spdx",
            spdx::check_spdx(&files),
            plural(count(spdx::is_checked), "source file", "source files"),
        ),
        (
            "run-lines",
            runlines::check_run_lines(&files),
            plural(
                count(runlines::is_workflow),
                "workflow file",
                "workflow files",
            ),
        ),
        (
            "main-rs",
            coverage::check_main_rs(root, &members),
            plural(members.len(), "crate", "crates"),
        ),
        (
            "purity",
            purity::check_purity(root, &members),
            plural(members.len(), "crate", "crates"),
        ),
        (
            "direction",
            direction::check_direction(root, &members),
            plural(members.len(), "crate", "crates"),
        ),
        (
            "unsafe-scope",
            unsafe_scope::check_unsafe_scope(root, &members),
            plural(members.len(), "manifest", "manifests"),
        ),
        (
            "public-lint tree",
            lint::check_tree(&files),
            format!("{file_count} scanned"),
        ),
        (
            "public-lint pull request text",
            lint::check_pr_text(event),
            "title and body scanned".to_owned(),
        ),
    ];
    gates.extend(commit_gates(root, runner, event, flag));
    gates
}

/// The exit code and the report for a set of gates.
pub fn render(gates: &[Gate]) -> (i32, String) {
    let mut text = String::new();
    let mut failed = 0;
    for (name, findings, examined) in gates {
        if findings.is_empty() {
            text.push_str(&format!("== {name}: ok ({examined})\n"));
        } else {
            failed += 1;
            text.push_str(&format!("== {name}: FAILED ({examined})\n"));
            for f in findings {
                text.push_str(&format!("{f}\n"));
            }
        }
    }
    text.push_str(&format!(
        "== xtask check: {failed} of {} gates failed\n",
        gates.len()
    ));
    (i32::from(failed > 0), text)
}
