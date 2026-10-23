// SPDX-License-Identifier: Apache-2.0
//! `cargo xtask check`: the static gates, every one run even after another fails. Tests and
//! coverage are the other job (`cargo xtask coverage`).

use std::path::Path;

use crate::cmd::{Cmd, Runner};
use crate::event::{Event, commit_base};
use crate::{Finding, coverage, dco, fetch, lint, runlines, spdx, tree};

/// One gate's name and what it found.
pub type Gate = (&'static str, Vec<Finding>);

fn command_gate(runner: &dyn Runner, cmd: &Cmd) -> Vec<Finding> {
    match runner.run(cmd) {
        Ok(_) => Vec::new(),
        Err(e) => vec![Finding::new("command", cmd.to_string(), e)],
    }
}

/// The commit-range gates: sign-off and the public lint over the commit messages.
fn commit_gates(root: &Path, runner: &dyn Runner, event: &Event, flag: Option<&str>) -> Vec<Gate> {
    match commit_base(event, flag) {
        Err(finding) => vec![("dco", vec![finding])],
        Ok(None) => {
            eprintln!("== dco and commit-message lint: skipped (no pull request, no --base)");
            Vec::new()
        }
        Ok(Some(base)) => match dco::commits_since(root, runner, &base) {
            Err(e) => vec![("dco", vec![Finding::new("git", &base, e)])],
            Ok(commits) => vec![
                ("dco", dco::check_dco(&commits)),
                ("commit-message lint", lint::check_commits(&commits)),
            ],
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
    let mut gates: Vec<Gate> = vec![
        ("tools", fetch::ensure_all(root, runner)),
        ("tree", read_error),
        ("fmt", command_gate(runner, &fmt)),
        ("clippy", command_gate(runner, &clippy)),
        ("deny", command_gate(runner, &deny)),
        ("spdx", spdx::check_spdx(&files)),
        ("run-lines", runlines::check_run_lines(&files)),
        ("main-rs", coverage::check_main_rs(root, &members)),
        ("public-lint tree", lint::check_tree(&files)),
        ("public-lint pull request text", lint::check_pr_text(event)),
    ];
    gates.extend(commit_gates(root, runner, event, flag));
    gates
}

/// The exit code and the report for a set of gates.
pub fn render(gates: &[Gate]) -> (i32, String) {
    let mut text = String::new();
    let mut failed = 0;
    for (name, findings) in gates {
        if findings.is_empty() {
            text.push_str(&format!("== {name}: ok\n"));
        } else {
            failed += 1;
            text.push_str(&format!("== {name}: FAILED\n"));
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
