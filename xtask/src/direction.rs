// SPDX-License-Identifier: Apache-2.0
//! The dependency-direction check. Two rules, read from the crates' manifests:
//!
//! - the planning and provider crates sit below the ledger and the engine, so none of them
//!   may depend on a crate above it (`LAYERS`);
//! - a crate marked pure (see [`crate::purity`]) never depends on a workspace crate that is
//!   not marked pure, so purity cannot be lost through a dependency.
//!
//! Every dependency section counts: normal, development and build, plain or under a
//! `[target.<selector>]` table. A renamed dependency counts as the package it names.

use std::collections::{BTreeMap, BTreeSet};
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

/// What a manifest line sits under.
enum Section {
    Other,
    /// A list of dependencies, one per line.
    List,
    /// The table of one dependency, named by its key.
    Table(String),
}

/// The text after a `target.<selector>.` prefix, where the selector is a triple or a quoted
/// `cfg(...)` expression that may itself contain dots.
fn after_target(text: &str) -> &str {
    let end = match text.chars().next() {
        Some(quote @ ('\'' | '"')) => text[1..].find(quote).map(|at| at + 2),
        _ => text.find('.'),
    };
    end.and_then(|at| text[at..].strip_prefix('.'))
        .unwrap_or("")
}

/// The section a table header opens, for the plain and the target-specific forms of the
/// normal, development and build dependencies.
fn section_of(header: &str) -> Section {
    let rest = header.strip_prefix("target.").map_or(header, after_target);
    for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if rest == kind {
            return Section::List;
        }
        if let Some(name) = rest.strip_prefix(kind).and_then(|r| r.strip_prefix('.')) {
            return Section::Table(name.trim_matches(['"', '\'']).to_owned());
        }
    }
    Section::Other
}

/// The quoted string a `package` key in `text` is set to, when there is one: the inline form
/// `{ package = "name" }` and the table-line form `package = "name"` both end up here.
fn package_in(text: &str) -> Option<String> {
    let (at, _) = text.match_indices("package").find(|(at, key)| {
        let before = text[..*at].chars().next_back();
        let after = text[at + key.len()..].trim_start();
        before.is_none_or(|c| matches!(c, '{' | ',' | ' ' | '\t')) && after.starts_with('=')
    })?;
    let value = text[at + "package".len()..]
        .trim_start()
        .strip_prefix('=')?
        .trim_start();
    let quote = value.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    value[1..].split(quote).next().map(str::to_owned)
}

/// The package names a manifest depends on, in any dependency section: plain or
/// target-specific (`[target.'cfg(...)'.dependencies]`), normal, development or build, in the
/// list form, the table form `[dependencies.name]` and the dotted form `name.workspace = true`.
/// A dependency renamed with `package = "..."` is reported under the package it names, since
/// that is the crate it resolves to.
pub fn dependencies(manifest: &str) -> BTreeSet<String> {
    // Each key as written, with the package it is renamed to, when it is.
    let mut keys: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut section = Section::Other;
    for line in manifest.lines().map(str::trim) {
        if let Some(header) = line.strip_prefix('[') {
            section = section_of(header.trim_end_matches(']'));
            if let Section::Table(name) = &section {
                keys.entry(name.clone()).or_default();
            }
        } else if line.starts_with('#') {
            continue;
        } else if let Section::Table(name) = &section {
            if let Some(package) = package_in(line) {
                keys.insert(name.clone(), Some(package));
            }
        } else if matches!(section, Section::List)
            && let Some((key, value)) = line.split_once('=')
        {
            let mut parts = key.trim().split('.');
            let name = parts.next().unwrap_or_default().to_owned();
            let renamed = if parts.next() == Some("package") {
                package_in(&format!("package ={value}"))
            } else {
                package_in(value)
            };
            let slot = keys.entry(name).or_default();
            if renamed.is_some() {
                *slot = renamed;
            }
        }
    }
    keys.into_iter()
        .map(|(key, package)| package.unwrap_or(key))
        .collect()
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
