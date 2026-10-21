// SPDX-License-Identifier: Apache-2.0
//! `deny.toml` refuses what it names. Each test runs the real cargo-deny with the committed
//! `deny.toml` on a small fixture workspace. A missing tool is a failure: the clean fixture
//! must pass first, so an absent tool cannot make a planted case look red.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{copy_dir, fixtures, git, root, scratch};
use xtask::deny::deny_check;

const OFFLINE_CHECKS: &[&str] = &["licenses", "bans", "sources"];

fn run_on(dir: &Path) -> Result<(), String> {
    deny_check(
        &dir.join("Cargo.toml"),
        &root().join("deny.toml"),
        OFFLINE_CHECKS,
    )
    .map_err(|f| {
        f.into_iter()
            .map(|f| f.detail)
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn fixture(case: &str, name: &str) -> PathBuf {
    let dir = scratch(&format!("deny-{case}"));
    copy_dir(&fixtures().join("deny").join(name), &dir);
    dir
}

#[test]
fn clean_fixture_passes() {
    let dir = fixture("clean", "clean");
    run_on(&dir).unwrap_or_else(|e| panic!("clean fixture must pass:\n{e}"));
}

#[test]
fn planted_copyleft_licence_red() {
    run_on(&fixture("control-copyleft", "clean")).expect("control passes");
    let err =
        run_on(&fixture("copyleft", "copyleft")).expect_err("a GPL dependency must be refused");
    assert!(err.contains("GPL-3.0-only"), "{err}");
}

#[test]
fn planted_banned_crate_red() {
    run_on(&fixture("control-banned", "clean")).expect("control passes");
    let err = run_on(&fixture("banned", "banned")).expect_err("a banned crate must be refused");
    assert!(
        err.contains("openssl-sys") && err.contains("banned"),
        "{err}"
    );
}

#[test]
fn planted_unknown_registry_red() {
    run_on(&fixture("control-source", "clean")).expect("control passes");
    // A dependency from a git repository is a source other than crates.io. The repository is
    // made here, on disk, so the test needs no network.
    let dir = fixture("source", "git_source");
    let repo = scratch("deny-source-repo");
    copy_dir(&dir.join("dep"), &repo);
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "fixture"]);
    let url = format!(
        "file://{}",
        repo.canonicalize().expect("repo path").display()
    );
    let manifest = fs::read_to_string(dir.join("Cargo.toml.in")).expect("template");
    fs::write(dir.join("Cargo.toml"), manifest.replace("GIT_URL", &url)).expect("manifest");
    let err = run_on(&dir).expect_err("a git source must be refused");
    assert!(
        err.contains("git_dep") && err.to_lowercase().contains("source"),
        "{err}"
    );
}
