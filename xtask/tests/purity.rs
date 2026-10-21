// SPDX-License-Identifier: Apache-2.0
mod common;

use std::fs;

use xtask::deps::Workspace;
use xtask::purity::check_purity;

const TEMPLATE: &str = "allow-unwrap-in-tests = true\n";

/// One crate under a scratch root; `pure` marks it; `clippy` is its clippy.toml, if any.
fn workspace(case: &str, pure: bool, clippy: Option<&str>) -> Workspace {
    let dir = common::scratch(&format!("purity-{case}"));
    if let Some(c) = clippy {
        fs::write(dir.join("clippy.toml"), c).expect("write");
    }
    let meta = if pure {
        r#"{"ucdk": {"pure": true}}"#
    } else {
        "null"
    };
    let json = format!(
        r#"{{"packages": [{{"name": "c", "manifest_path": "{}", "metadata": {meta}, "dependencies": []}}]}}"#,
        dir.join("Cargo.toml").display()
    );
    Workspace::from_metadata_json(&json).expect("parse")
}

#[test]
fn crate_without_template_clippy_toml_flagged() {
    let f = check_purity(&workspace("missing", true, None), TEMPLATE);
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].rule, "purity-missing");
}

#[test]
fn drifted_clippy_toml_flagged() {
    let f = check_purity(
        &workspace("drift", true, Some("allow-unwrap-in-tests = false\n")),
        TEMPLATE,
    );
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].rule, "purity-drift");
}

#[test]
fn matching_clippy_toml_ok() {
    assert_eq!(
        check_purity(&workspace("ok", true, Some(TEMPLATE)), TEMPLATE),
        vec![]
    );
}

#[test]
fn non_pure_crate_ignored() {
    assert_eq!(
        check_purity(&workspace("impure", false, None), TEMPLATE),
        vec![]
    );
}
