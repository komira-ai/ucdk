// SPDX-License-Identifier: Apache-2.0
//! The coverage gate. `cargo xtask coverage` runs the whole test suite once under `cargo llvm-cov`
//! on the pinned nightly (branch coverage needs it), reads the lcov file itself and holds every
//! crate to the floor for lines and for branches, each crate on its own.
//!
//! Left out of the measurement: generated code (anything under a `target` directory, where
//! prost output lands) and a `main.rs`. A `main.rs` is only allowed to be a forwarding stub: more
//! than `MAIN_RS_LINES` lines after the licence header is a finding.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::cmd::{Cmd, Runner};
use crate::{Finding, fetch, io_err};

/// The least percentage of lines, and of branches, each crate must cover.
pub const FLOOR: u64 = 99;
/// The nightly the coverage job uses; the only place a nightly appears.
pub const NIGHTLY: &str = "nightly-2026-10-01";
/// The longest a `main.rs` may be, not counting the licence header line.
pub const MAIN_RS_LINES: usize = 3;
pub const LCOV: &str = "target/xtask-coverage/lcov.info";
const LCOV_DIR: &str = "target/xtask-coverage";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub lines: u64,
    pub lines_hit: u64,
    pub branches: u64,
    pub branches_hit: u64,
}

/// The crate a source path belongs to: `crates/<name>/...` is `<name>`, otherwise the first
/// directory (`xtask`). `root` is stripped first when the path is absolute.
pub fn crate_of(path: &str, root: &str) -> String {
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .trim_start_matches('/');
    let mut parts = rel.split('/');
    let first = parts.next().unwrap_or("");
    let name = if first == "crates" {
        parts.next().unwrap_or("")
    } else {
        first
    };
    name.to_owned()
}

fn relative<'a>(path: &'a str, root: &str) -> &'a str {
    path.strip_prefix(root)
        .unwrap_or(path)
        .trim_start_matches('/')
}

/// `path` is relative to the workspace root, so a checkout that itself sits under a `target`
/// directory is still measured.
fn excluded(path: &str) -> bool {
    path.ends_with("/main.rs") || path.split('/').any(|part| part == "target")
}

/// Sums the `DA:` (line) and `BRDA:` (branch) records of an lcov file per crate.
pub fn parse_lcov(text: &str, root: &str) -> BTreeMap<String, Counts> {
    let mut out: BTreeMap<String, Counts> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("SF:") {
            current = Some(path.to_owned()).filter(|p| !excluded(relative(p, root)));
        } else if let Some(name) = current.as_deref().map(|p| crate_of(p, root)) {
            let counts = out.entry(name).or_default();
            if let Some(da) = line.strip_prefix("DA:") {
                counts.lines += 1;
                counts.lines_hit += u64::from(taken(da.split(',').nth(1)));
            } else if let Some(br) = line.strip_prefix("BRDA:") {
                counts.branches += 1;
                counts.branches_hit += u64::from(taken(br.split(',').nth(3)));
            }
        }
    }
    out
}

/// Whether an lcov count field says "executed at least once" (`-` and `0` do not).
fn taken(field: Option<&str>) -> bool {
    field
        .and_then(|f| f.trim().parse::<u64>().ok())
        .unwrap_or(0)
        > 0
}

fn percent(hit: u64, total: u64) -> String {
    let hundredths = (hit * 10_000).checked_div(total).unwrap_or(0);
    format!("{}.{:02}%", hundredths / 100, hundredths % 100)
}

/// Holds every crate in `members` (and any other crate the file measured) to `floor`. Returns
/// the findings and one report line per crate.
pub fn gate(
    counts: &BTreeMap<String, Counts>,
    members: &[String],
    floor: u64,
) -> (Vec<Finding>, Vec<String>) {
    let names: BTreeSet<&String> = members.iter().chain(counts.keys()).collect();
    let (mut findings, mut report) = (Vec::new(), Vec::new());
    for name in names {
        let c = counts.get(name).copied().unwrap_or_default();
        report.push(format!(
            "{name}: lines {} ({}/{}), branches {} ({}/{})",
            percent(c.lines_hit, c.lines),
            c.lines_hit,
            c.lines,
            percent(c.branches_hit, c.branches),
            c.branches_hit,
            c.branches
        ));
        if c.lines == 0 {
            findings.push(Finding::new(
                "coverage-no-lines",
                name,
                "no line of this crate was measured",
            ));
        } else if c.lines_hit * 100 < floor * c.lines {
            findings.push(Finding::new(
                "coverage-lines",
                name,
                format!(
                    "line coverage {} is below {floor}%",
                    percent(c.lines_hit, c.lines)
                ),
            ));
        }
        if c.branches_hit * 100 < floor * c.branches {
            findings.push(Finding::new(
                "coverage-branches",
                name,
                format!(
                    "branch coverage {} is below {floor}%",
                    percent(c.branches_hit, c.branches)
                ),
            ));
        }
    }
    (findings, report)
}

/// The workspace members: every directory under `crates/`, and `xtask`.
pub fn members(root: &Path) -> Result<Vec<String>, String> {
    let mut out = vec!["xtask".to_owned()];
    for entry in fs::read_dir(root.join("crates")).map_err(io_err)? {
        let entry = entry.map_err(io_err)?;
        if entry.path().is_dir() {
            out.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    out.sort();
    Ok(out)
}

/// A finding for every `main.rs` that is more than a stub.
pub fn check_main_rs(root: &Path, members: &[String]) -> Vec<Finding> {
    let mut out = Vec::new();
    for name in members {
        let dir = if name == "xtask" {
            root.join("xtask")
        } else {
            root.join("crates").join(name)
        };
        let Ok(text) = fs::read_to_string(dir.join("src/main.rs")) else {
            continue;
        };
        let code = text
            .lines()
            .skip(1)
            .filter(|l| !l.trim().is_empty())
            .count();
        if code > MAIN_RS_LINES {
            out.push(Finding::new(
                "main-rs-too-long",
                format!("{name}/src/main.rs"),
                format!("{code} lines after the header; a main.rs forwards to the library in at most {MAIN_RS_LINES}"),
            ));
        }
    }
    out
}

/// The verb: install the tools, run the tests under llvm-cov, gate the lcov file.
pub fn run(root: &Path, runner: &dyn Runner) -> (Vec<Finding>, Vec<String>) {
    let mut findings = fetch::ensure_all(root, runner);
    if !findings.is_empty() {
        return (findings, Vec::new());
    }
    if let Err(e) = fs::create_dir_all(root.join(LCOV_DIR)) {
        return (
            vec![Finding::new("coverage-run", LCOV_DIR, e.to_string())],
            Vec::new(),
        );
    }
    let bin = fetch::bin_dir(root);
    let rustup = Cmd::new(
        "rustup",
        &[
            "toolchain",
            "install",
            NIGHTLY,
            "--profile",
            "minimal",
            "--component",
            "llvm-tools-preview",
            "--no-self-update",
        ],
        root,
    );
    let toolchain = format!("+{NIGHTLY}");
    let llvm_cov = Cmd::new(
        "cargo",
        &[
            &toolchain,
            "llvm-cov",
            "--branch",
            "--workspace",
            "--locked",
            "--lcov",
            "--output-path",
            LCOV,
        ],
        root,
    )
    .with_path(&bin);
    for cmd in [rustup, llvm_cov] {
        if let Err(e) = runner.run(&cmd) {
            return (
                vec![Finding::new("coverage-run", cmd.to_string(), e)],
                Vec::new(),
            );
        }
    }
    let measured = fs::read_to_string(root.join(LCOV)).map_err(io_err);
    let names = members(root);
    match (measured, names) {
        (Ok(text), Ok(names)) => {
            let root_text = root
                .canonicalize()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let (found, report) = gate(&parse_lcov(&text, &root_text), &names, FLOOR);
            findings.extend(found);
            findings.extend(check_main_rs(root, &names));
            (findings, report)
        }
        (Err(e), _) | (_, Err(e)) => (vec![Finding::new("coverage-read", LCOV, e)], Vec::new()),
    }
}
