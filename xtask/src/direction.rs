// SPDX-License-Identifier: Apache-2.0
//! The dependency-direction check. Two rules, read from the crates' manifests:
//!
//! - the planning and provider crates sit below the ledger and the engine, so none of them
//!   may depend on a crate above it (`LAYERS`);
//! - a crate marked pure (see [`crate::purity`]) never depends on a workspace crate that is
//!   not marked pure, so purity cannot be lost through a dependency.
//!
//! Every dependency section counts: normal, development and build.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::{Finding, purity};

/// Each crate, with the crates it must never depend on.
pub const LAYERS: [(&str, &[&str]); 4] = [
    ("ucdk-provider", &["ucdk-ledger", "ucdk-engine"]),
    ("ucdk-plan", &["ucdk-ledger", "ucdk-engine"]),
    ("ucdk-provider-fake", &["ucdk-ledger", "ucdk-engine"]),
    ("ucdk-ledger", &["ucdk-engine"]),
];

/// The names a manifest depends on, in any dependency section, including the table form
/// `[dependencies.name]` and the dotted form `name.workspace = true`.
pub fn dependencies(manifest: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut in_list = false;
    for line in manifest.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[') {
            let header = header.trim_end_matches(']');
            let table = ["dependencies.", "dev-dependencies.", "build-dependencies."]
                .iter()
                .find_map(|prefix| header.strip_prefix(prefix));
            if let Some(name) = table {
                found.insert(name.to_owned());
            }
            in_list = matches!(
                header,
                "dependencies" | "dev-dependencies" | "build-dependencies"
            );
        } else if in_list
            && !line.starts_with('#')
            && let Some((key, _)) = line.split_once('=')
        {
            let name = key.trim().split('.').next().unwrap_or_default();
            found.insert(name.to_owned());
        }
    }
    found
}

/// A finding for each dependency that breaks a rule, over the crates of `members` under `root`.
pub fn check_direction(root: &Path, members: &[String]) -> Vec<Finding> {
    let manifest =
        |name: &str| fs::read_to_string(root.join("crates").join(name).join("Cargo.toml"));
    let crates: Vec<(&String, String)> = members
        .iter()
        .filter(|n| *n != "xtask")
        .filter_map(|n| manifest(n).ok().map(|m| (n, m)))
        .collect();
    let pure: BTreeSet<&str> = crates
        .iter()
        .filter(|(_, m)| purity::is_pure(m))
        .map(|(n, _)| n.as_str())
        .collect();
    let mut out = Vec::new();
    for (name, text) in &crates {
        let deps = dependencies(text);
        let banned = LAYERS
            .iter()
            .find(|(layer, _)| layer == name)
            .map_or(&[][..], |(_, banned)| *banned);
        for dep in deps.iter().filter(|d| banned.contains(&d.as_str())) {
            out.push(Finding::new(
                "dependency-direction",
                format!("{name}/Cargo.toml"),
                format!("{name} sits below {dep} and must not depend on it"),
            ));
        }
        if pure.contains(name.as_str()) {
            for dep in deps
                .iter()
                .filter(|d| members.contains(d) && !pure.contains(d.as_str()))
            {
                out.push(Finding::new(
                    "pure-depends-on-impure",
                    format!("{name}/Cargo.toml"),
                    format!("{name} is pure but depends on {dep}, which is not marked pure"),
                ));
            }
        }
    }
    out
}
