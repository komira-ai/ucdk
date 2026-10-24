// SPDX-License-Identifier: Apache-2.0
//! The purity template. A crate marked pure holds no I/O, no wall clock and no `HashMap`, so
//! its results depend only on its inputs. The mark is `pure = true` under
//! `[package.metadata.ucdk]` in its `Cargo.toml`; the template is a `clippy.toml` next to that
//! manifest that denies each of the entries below, so clippy refuses the code that would
//! break the promise. A crate-level `clippy.toml` replaces the workspace one, so the template
//! must also repeat the test allowances.

use std::fs;
use std::path::Path;

use crate::Finding;

/// Entries the template must deny, by clippy setting.
pub const REQUIRED: [(&str, &[&str]); 3] = [
    (
        "disallowed-types",
        &[
            "std::collections::HashMap",
            "std::collections::HashSet",
            "std::time::SystemTime",
            "std::time::Instant",
            "std::fs::File",
            "std::net::TcpStream",
        ],
    ),
    (
        "disallowed-methods",
        &[
            "std::time::SystemTime::now",
            "std::time::Instant::now",
            "std::fs::read",
            "std::fs::read_to_string",
            "std::fs::write",
            "std::fs::read_dir",
            "std::net::TcpStream::connect",
            "std::process::Command::new",
            "std::env::var",
            "std::thread::sleep",
        ],
    ),
    (
        "disallowed-macros",
        &["std::println", "std::eprintln", "std::print", "std::eprint"],
    ),
];

/// The test allowances a crate-level `clippy.toml` must repeat.
pub const ALLOWANCES: [&str; 2] = [
    "allow-unwrap-in-tests = true",
    "allow-expect-in-tests = true",
];

/// Whether a manifest marks its crate pure.
pub fn is_pure(manifest: &str) -> bool {
    let mut in_section = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_section = line == "[package.metadata.ucdk]";
        } else if in_section && line.split('#').next().map(str::trim) == Some("pure = true") {
            return true;
        }
    }
    false
}

/// The part of `template` that follows the setting `name`, up to the next top-level setting.
fn section<'a>(template: &'a str, name: &str) -> &'a str {
    let Some(start) = template.find(&format!("{name} = [")) else {
        return "";
    };
    let rest = &template[start..];
    rest.find("\n]").map_or(rest, |end| &rest[..end])
}

/// What a template lacks, one entry per line of the result.
pub fn missing(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    for allowance in ALLOWANCES {
        if !template.lines().any(|l| l.trim() == allowance) {
            out.push(allowance.to_owned());
        }
    }
    for (setting, entries) in REQUIRED {
        let body = section(template, setting);
        for entry in entries {
            if !body.contains(&format!("path = \"{entry}\"")) {
                out.push(format!("{setting}: {entry}"));
            }
        }
    }
    out
}

/// A finding for every pure crate (of `members`, under `root`) whose `clippy.toml` is missing
/// or does not carry the whole template.
pub fn check_purity(root: &Path, members: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    for name in members.iter().filter(|n| *n != "xtask") {
        let dir = root.join("crates").join(name);
        let pure = fs::read_to_string(dir.join("Cargo.toml")).is_ok_and(|m| is_pure(&m));
        if !pure {
            continue;
        }
        let subject = format!("{name}/clippy.toml");
        match fs::read_to_string(dir.join("clippy.toml")) {
            Err(_) => out.push(Finding::new(
                "purity-template-missing",
                subject,
                "a crate marked pure carries the deny-list clippy.toml (wall clock, HashMap, I/O)",
            )),
            Ok(template) => {
                let lacking = missing(&template);
                if !lacking.is_empty() {
                    out.push(Finding::new(
                        "purity-template-incomplete",
                        subject,
                        format!("lacks {}", lacking.join(", ")),
                    ));
                }
            }
        }
    }
    out
}
