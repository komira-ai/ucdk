// SPDX-License-Identifier: Apache-2.0
use std::process::ExitCode;

use xtask::{Finding, check, dco, deny, deps, purity, spdx, tools, workflowlint, workspace_root};

const USAGE: &str = "usage: cargo xtask <verb>
  check [--base <sha>] [--install-tools]   every gate
  deps | purity | spdx | workflowlint | deny
  dco --base <sha>
  tools                                    install the pinned external tools";

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn report(result: Result<(), Vec<Finding>>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(findings) => {
            for f in findings {
                eprintln!("{f}");
            }
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = workspace_root();
    match args.first().map(String::as_str) {
        Some("check") => {
            let base = flag_value(&args, "--base");
            let install = args.iter().any(|a| a == "--install-tools");
            if check::run(&root, base.as_deref(), install) == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Some("deps") => report(deps::run(&root)),
        Some("purity") => report(purity::run(&root)),
        Some("spdx") => report(spdx::run(&root)),
        Some("workflowlint") => report(workflowlint::run(&root)),
        Some("deny") => report(deny::run(&root)),
        Some("tools") => report(tools::run(true)),
        Some("dco") => match flag_value(&args, "--base") {
            Some(base) => report(dco::run(&root, &base)),
            None => {
                eprintln!("{USAGE}");
                ExitCode::from(2)
            }
        },
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
