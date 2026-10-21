// SPDX-License-Identifier: Apache-2.0
//! The purity template really disables what it names: clippy is run on a fixture crate that
//! carries a copy of the template and one planted call, and must refuse it by lint name.

mod common;

use std::fs;
use std::process::Command;

use common::{cargo, fixtures, root, scratch, text};

/// Builds the fixture crate with `variant` as its lib.rs and the *current* template as its
/// clippy.toml, runs clippy with warnings denied, and returns (success, output).
fn clippy_on(case: &str, variant: &str) -> (bool, String) {
    let dir = scratch(&format!("selftest-{case}"));
    let fx = fixtures().join("pure_crate");
    fs::copy(fx.join("Cargo.toml"), dir.join("Cargo.toml")).expect("copy manifest");
    fs::create_dir_all(dir.join("src")).expect("src");
    fs::copy(fx.join(variant), dir.join("src/lib.rs")).expect("copy variant");
    fs::copy(
        root().join("xtask/templates/pure_clippy.toml"),
        dir.join("clippy.toml"),
    )
    .expect("template");
    let out = Command::new(cargo())
        .args(["clippy", "--all-targets", "--", "-D", "warnings"])
        .current_dir(&dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .expect("run cargo clippy");
    (out.status.success(), text(&out))
}

fn assert_refused(case: &str, variant: &str, lint: &str) {
    let (ok, out) = clippy_on(case, variant);
    assert!(!ok, "{variant} must fail clippy, got success:\n{out}");
    assert!(
        out.contains(lint),
        "{variant} must be refused by {lint}:\n{out}"
    );
}

#[test]
fn planted_hashmap_in_pure_crate_fails_clippy() {
    assert_refused("hashmap", "hashmap.rs", "clippy::disallowed_types");
}

#[test]
fn planted_systemtime_now_fails_clippy() {
    assert_refused("systemtime", "systemtime.rs", "clippy::disallowed_methods");
}

#[test]
fn planted_env_var_fails_clippy() {
    assert_refused("env_var", "env_var.rs", "clippy::disallowed_methods");
}

#[test]
fn planted_file_open_fails_clippy() {
    assert_refused("file_open", "file_open.rs", "clippy::disallowed_methods");
}

#[test]
fn planted_command_new_fails_clippy() {
    assert_refused(
        "command_new",
        "command_new.rs",
        "clippy::disallowed_methods",
    );
}

#[test]
fn unplanted_fixture_passes_clippy() {
    let (ok, out) = clippy_on("valid", "valid.rs");
    assert!(ok, "valid code must pass the template:\n{out}");
}
