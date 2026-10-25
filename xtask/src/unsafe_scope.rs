// SPDX-License-Identifier: Apache-2.0
//! The `unsafe` exemption. The workspace forbids `unsafe` code (`unsafe_code = "forbid"` in the
//! root manifest) and every member inherits that. `ucdk-capi`, the C ABI, is the one crate that
//! needs it, so its manifest lowers the lint to `deny` and its `ffi` module lifts the denial.
//! This check pins that shape from the manifests:
//!
//! - the root manifest still forbids `unsafe_code`;
//! - every member but the exempt crate inherits the workspace lints (`[lints] workspace = true`)
//!   and names `unsafe_code` nowhere;
//! - the exempt crate sets `unsafe_code = "deny"`, no weaker.

use std::fs;
use std::path::Path;

use crate::Finding;

/// The one crate whose manifest may set `unsafe_code`.
pub const EXEMPT: &str = "ucdk-capi";

/// The lines of a manifest without comments or blanks, trimmed.
fn lines(manifest: &str) -> impl Iterator<Item = &str> {
    manifest
        .lines()
        .filter_map(|l| l.split('#').next())
        .map(str::trim)
        .filter(|l| !l.is_empty())
}

/// The value `unsafe_code` is set to in a manifest, with its quotes removed.
fn unsafe_code_setting(manifest: &str) -> Option<String> {
    lines(manifest).find_map(|l| {
        let (key, value) = l.split_once('=')?;
        (key.trim() == "unsafe_code").then(|| value.trim().trim_matches('"').to_owned())
    })
}

/// Whether the manifest inherits the workspace lint table.
fn inherits_lints(manifest: &str) -> bool {
    let mut in_lints = false;
    for line in lines(manifest) {
        if line.starts_with('[') {
            in_lints = line == "[lints]";
        } else if in_lints && line.replace(' ', "") == "workspace=true" {
            return true;
        }
    }
    false
}

fn manifest_path(root: &Path, name: &str) -> std::path::PathBuf {
    if name == "xtask" {
        root.join("xtask/Cargo.toml")
    } else {
        root.join("crates").join(name).join("Cargo.toml")
    }
}

/// A finding for every manifest that breaks the shape above, over the root and `members`.
pub fn check_unsafe_scope(root: &Path, members: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    match fs::read_to_string(root.join("Cargo.toml")) {
        Ok(text) if unsafe_code_setting(&text).as_deref() == Some("forbid") => {}
        Ok(_) => out.push(Finding::new(
            "unsafe-not-forbidden",
            "Cargo.toml",
            "the workspace must set unsafe_code = \"forbid\"",
        )),
        Err(e) => out.push(Finding::new(
            "unsafe-unreadable",
            "Cargo.toml",
            e.to_string(),
        )),
    }
    for name in members {
        let subject = format!("{name}/Cargo.toml");
        let text = match fs::read_to_string(manifest_path(root, name)) {
            Ok(text) => text,
            Err(e) => {
                out.push(Finding::new("unsafe-unreadable", subject, e.to_string()));
                continue;
            }
        };
        let setting = unsafe_code_setting(&text);
        if name == EXEMPT {
            if setting.as_deref() != Some("deny") {
                out.push(Finding::new(
                    "unsafe-exemption-not-deny",
                    subject,
                    format!("{EXEMPT} sets unsafe_code = \"deny\" and nothing else"),
                ));
            }
        } else if setting.is_some() {
            out.push(Finding::new(
                "unsafe-override",
                subject,
                format!("only {EXEMPT} may set unsafe_code"),
            ));
        } else if !inherits_lints(&text) {
            out.push(Finding::new(
                "lints-not-inherited",
                subject,
                "a crate inherits the workspace lints ([lints] workspace = true)",
            ));
        }
    }
    out
}
