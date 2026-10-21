// SPDX-License-Identifier: Apache-2.0
//! The external tools `check` needs, each pinned to one version.

use std::process::Command;

use crate::{Finding, to_result};

pub struct Tool {
    /// The crate to `cargo install`.
    pub krate: &'static str,
    /// The cargo subcommand it provides.
    pub subcommand: &'static str,
    pub version: &'static str,
}

/// Name and version in one table.
pub const TOOLS: &[Tool] = &[Tool {
    krate: "cargo-deny",
    subcommand: "deny",
    version: "0.20.2",
}];

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

fn installed(tool: &Tool) -> bool {
    Command::new(cargo())
        .args([tool.subcommand, "--version"])
        .output()
        .is_ok_and(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).contains(tool.version)
        })
}

/// With `install`, installs each missing or mismatched tool; without, reports it.
pub fn run(install: bool) -> Result<(), Vec<Finding>> {
    let mut findings = Vec::new();
    for tool in TOOLS.iter().filter(|t| !installed(t)) {
        if !install {
            findings.push(Finding::new(
                "tool-missing",
                tool.krate,
                format!(
                    "version {} is not installed; run `cargo xtask tools`",
                    tool.version
                ),
            ));
            continue;
        }
        let status = Command::new(cargo())
            .args(["install", "--locked", "--version", tool.version, tool.krate])
            .status();
        if !status.is_ok_and(|s| s.success()) {
            findings.push(Finding::new(
                "tool-install",
                tool.krate,
                "cargo install failed",
            ));
        }
    }
    to_result(findings)
}
