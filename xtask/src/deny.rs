// SPDX-License-Identifier: Apache-2.0
//! `cargo deny`: licences, advisories, sources and bans, as `deny.toml` configures them.

use std::path::Path;
use std::process::Command;

use crate::Finding;

/// Runs `cargo deny check <checks>` on `manifest` with `config`. A missing tool is a finding,
/// never a skip. The finding's detail is the tool's output.
pub fn deny_check(manifest: &Path, config: &Path, checks: &[&str]) -> Result<(), Vec<Finding>> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = Command::new(cargo)
        .args(["deny", "--log-level", "warn", "--manifest-path"])
        .arg(manifest)
        .arg("--all-features")
        .arg("--config")
        .arg(config)
        .arg("check")
        .args(checks)
        .output();
    match out {
        Err(e) => Err(vec![Finding::new("deny", "cargo deny", e.to_string())]),
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(vec![Finding::new(
            "deny",
            "cargo deny",
            format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            ),
        )]),
    }
}

/// The `deny` verb: every check on the real workspace.
pub fn run(root: &Path) -> Result<(), Vec<Finding>> {
    deny_check(&root.join("Cargo.toml"), &root.join("deny.toml"), &[])
}
