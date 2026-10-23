// SPDX-License-Identifier: Apache-2.0
//! The verbs: `check`, `coverage` and `fetch`.

use std::path::Path;
use std::process::ExitCode;

use crate::cmd::{Runner, System};
use crate::event::{Event, parse_event};
use crate::{Finding, check, coverage, fetch, workspace_root};

const USAGE: &str = "usage: cargo xtask <verb>
  check [--base <sha>]   fmt, clippy, deny, SPDX, run lines, public lint, DCO
  coverage               tests under llvm-cov; per-crate line and branch floor
  fetch                  install the pinned tools
";

/// The platform-set variable that names the pull-request event file.
const EVENT_PATH: &str = "GITHUB_EVENT_PATH";

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    let at = args.iter().position(|a| a == flag)?;
    args.get(at + 1).cloned()
}

fn findings_text(findings: &[Finding]) -> String {
    findings.iter().map(|f| format!("{f}\n")).collect()
}

fn load_event(path: Option<String>) -> Result<Event, String> {
    match path {
        Some(path) => parse_event(&std::fs::read_to_string(path).map_err(|e| e.to_string())?),
        None => Ok(Event::default()),
    }
}

/// Runs a verb: the exit code and what to print.
pub fn run(
    args: &[String],
    env: &dyn Fn(&str) -> Option<String>,
    runner: &dyn Runner,
    root: &Path,
) -> (i32, String) {
    let event_path = env(EVENT_PATH);
    match args.first().map(String::as_str) {
        Some("check") => match load_event(event_path) {
            Ok(event) => check::render(&check::run(
                root,
                runner,
                &event,
                flag_value(args, "--base").as_deref(),
            )),
            Err(e) => (1, format!("{e}\n")),
        },
        Some("coverage") => {
            let (findings, report) = coverage::run(root, runner);
            let mut text = report.join("\n");
            text.push('\n');
            text.push_str(&findings_text(&findings));
            (i32::from(!findings.is_empty()), text)
        }
        Some("fetch") => {
            let findings = fetch::ensure_all(root, runner);
            (i32::from(!findings.is_empty()), findings_text(&findings))
        }
        _ => (2, USAGE.to_owned()),
    }
}

/// The process entry point.
pub fn main_entry() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (code, text) = run(
        &args,
        &|name| std::env::var(name).ok(),
        &System,
        &workspace_root(),
    );
    eprint!("{text}");
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}
