// SPDX-License-Identifier: Apache-2.0
//! `cargo xtask check`: every gate, in order, all run even after one fails.

use std::path::Path;
use std::process::Command;

use crate::{Finding, dco, deny, deps, purity, spdx, tools, workflowlint};

fn cargo_step(root: &Path, args: &[&str]) -> Result<(), Vec<Finding>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let status = Command::new(cargo).args(args).current_dir(root).status();
    if status.is_ok_and(|s| s.success()) {
        Ok(())
    } else {
        Err(vec![Finding::new(
            "cargo",
            format!("cargo {}", args.join(" ")),
            "failed",
        )])
    }
}

/// What the DCO gate does for this run. A pull request must always be checked, so an empty
/// base there is an error; elsewhere (a local run, a push to main) the gate is skipped, loudly.
/// `event` is `GITHUB_EVENT_NAME`, a mode marker the platform sets.
pub fn dco_plan<'a>(
    base: Option<&'a str>,
    event: Option<&str>,
) -> Result<Option<&'a str>, Finding> {
    match base.filter(|b| !b.is_empty()) {
        Some(b) => Ok(Some(b)),
        None if event == Some("pull_request") => Err(Finding::new(
            "dco-no-base",
            "--base",
            "this is a pull request but no --base was given, so no commit would be checked",
        )),
        None => Ok(None),
    }
}

/// Runs every gate; `base` (when non-empty) also enables the DCO check over `base..HEAD`.
/// Returns the number of failed gates.
pub fn run(root: &Path, base: Option<&str>, install_tools: bool) -> usize {
    type Step<'a> = (&'a str, Box<dyn Fn() -> Result<(), Vec<Finding>> + 'a>);
    let mut steps: Vec<Step<'_>> = vec![
        ("tools", Box::new(move || tools::run(install_tools))),
        (
            "fmt",
            Box::new(|| cargo_step(root, &["fmt", "--all", "--check"])),
        ),
        (
            "clippy",
            Box::new(|| {
                cargo_step(
                    root,
                    &[
                        "clippy",
                        "--workspace",
                        "--all-targets",
                        "--locked",
                        "--",
                        "-D",
                        "warnings",
                    ],
                )
            }),
        ),
        (
            "test",
            Box::new(|| cargo_step(root, &["test", "--workspace", "--locked"])),
        ),
        ("deps", Box::new(|| deps::run(root))),
        ("purity", Box::new(|| purity::run(root))),
        ("spdx", Box::new(|| spdx::run(root))),
        ("workflowlint", Box::new(|| workflowlint::run(root))),
        ("deny", Box::new(|| deny::run(root))),
    ];
    match dco_plan(base, std::env::var("GITHUB_EVENT_NAME").ok().as_deref()) {
        Ok(Some(base)) => steps.push(("dco", Box::new(move || dco::run(root, base)))),
        Ok(None) => eprintln!("== dco: skipped (no --base)"),
        Err(finding) => steps.push(("dco", Box::new(move || Err(vec![finding.clone()])))),
    }
    let mut failed = 0;
    for (name, step) in &steps {
        eprintln!("== xtask check: {name}");
        if let Err(findings) = step() {
            failed += 1;
            for f in findings {
                eprintln!("{f}");
            }
            eprintln!("== {name}: FAILED");
        }
    }
    if failed == 0 {
        eprintln!("== xtask check: ok ({} gates)", steps.len());
    } else {
        eprintln!("== xtask check: {failed} of {} gates failed", steps.len());
    }
    failed
}
