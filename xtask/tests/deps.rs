// SPDX-License-Identifier: Apache-2.0
use serde_json::{Value, json};
use xtask::deps::{DepRules, Workspace, check_deps};

/// (name, pure, [(dependency, kind)]) -> a Workspace through the real parser.
type Pkg<'a> = (&'a str, bool, &'a [(&'a str, &'a str)]);

fn ws(pkgs: &[Pkg<'_>]) -> Workspace {
    let packages: Vec<Value> = pkgs
        .iter()
        .map(|(name, pure, deps)| {
            let deps: Vec<Value> = deps
                .iter()
                .map(|(d, kind)| json!({"name": d, "kind": if kind.is_empty() { Value::Null } else { json!(kind) }}))
                .collect();
            json!({
                "name": name,
                "manifest_path": format!("/w/crates/{name}/Cargo.toml"),
                "metadata": if *pure { json!({"ucdk": {"pure": true}}) } else { Value::Null },
                "dependencies": deps,
            })
        })
        .collect();
    Workspace::from_metadata_json(&json!({ "packages": packages }).to_string()).expect("parse")
}

fn rules() -> DepRules {
    DepRules::ucdk()
}

fn flagged(ws: &Workspace, rule: &str, subject: &str) -> bool {
    check_deps(ws, &rules())
        .iter()
        .any(|f| f.rule == rule && f.subject == subject)
}

#[test]
fn pure_depending_on_impure_is_flagged() {
    let w = ws(&[
        ("ucdk-plan", true, &[("ucdk-ledger", "")]),
        ("ucdk-ledger", false, &[]),
    ]);
    assert!(flagged(&w, "pure-depends-on-impure", "ucdk-plan"));
}

#[test]
fn api_with_extra_dependency_flagged() {
    let w = ws(&[
        ("ucdk-api", true, &[("ucdk-proto", ""), ("ucdk-plan", "")]),
        ("ucdk-proto", true, &[]),
        ("ucdk-plan", true, &[]),
    ]);
    assert!(flagged(&w, "only-deps", "ucdk-api"));
}

#[test]
fn api_with_allowed_dependencies_ok() {
    let w = ws(&[
        ("ucdk-api", true, &[("ucdk-proto", ""), ("ucdk-model", "")]),
        ("ucdk-proto", true, &[]),
        ("ucdk-model", true, &[]),
    ]);
    assert_eq!(check_deps(&w, &rules()), vec![]);
}

#[test]
fn cli_depending_on_engine_flagged() {
    let w = ws(&[
        ("ucdk-cli", false, &[("ucdk-engine", "")]),
        ("ucdk-engine", false, &[]),
    ]);
    assert!(flagged(&w, "only-via", "ucdk-cli"));
}

#[test]
fn cli_depending_on_dispatch_ok() {
    let w = ws(&[
        ("ucdk-cli", false, &[("ucdk-dispatch", "")]),
        ("ucdk-dispatch", false, &[]),
    ]);
    assert_eq!(check_deps(&w, &rules()), vec![]);
}

#[test]
fn capi_depending_on_publish_crate_flagged() {
    let w = ws(&[
        ("ucdk-capi", false, &[("ucdk-channel-apt", "")]),
        ("ucdk-channel-apt", false, &[]),
    ]);
    assert!(flagged(&w, "only-via", "ucdk-capi"));
}

#[test]
fn rules_naming_absent_crate_are_vacuous() {
    // None of the crates the rules name exist, so nothing binds.
    let w = ws(&[("ucdk-cli", false, &[])]);
    assert_eq!(check_deps(&w, &rules()), vec![]);
}

#[test]
fn dev_dependency_edge_counts() {
    let w = ws(&[
        ("ucdk-plan", true, &[("ucdk-ledger", "dev")]),
        ("ucdk-ledger", false, &[]),
    ]);
    assert!(flagged(&w, "pure-depends-on-impure", "ucdk-plan"));
}

#[test]
fn testutil_as_normal_dependency_flagged() {
    let w = ws(&[
        ("ucdk-plan", true, &[("ucdk-testutil", "")]),
        ("ucdk-testutil", true, &[]),
    ]);
    assert!(flagged(&w, "dev-only", "ucdk-plan"));
    let w = ws(&[
        ("ucdk-plan", true, &[("ucdk-testutil", "dev")]),
        ("ucdk-testutil", true, &[]),
    ]);
    assert_eq!(check_deps(&w, &rules()), vec![]);
}

#[test]
fn listed_pure_crate_without_marker_flagged() {
    let w = ws(&[("ucdk-model", false, &[])]);
    assert!(flagged(&w, "pure-unmarked", "ucdk-model"));
}

#[test]
fn real_workspace_is_clean() {
    let w = Workspace::load(&xtask::workspace_root()).expect("cargo metadata");
    assert!(
        w.packages.iter().any(|p| p.name == "xtask"),
        "the real workspace was read"
    );
    assert_eq!(check_deps(&w, &rules()), vec![]);
}
