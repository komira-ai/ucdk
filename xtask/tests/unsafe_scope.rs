// SPDX-License-Identifier: Apache-2.0
//! The `unsafe` exemption: the workspace forbids it, one crate may lower the lint to `deny`.

mod common;

use common::{scratch, write};
use xtask::coverage::members;
use xtask::unsafe_scope::check_unsafe_scope;
use xtask::workspace_root;

const FORBID: &str = "[workspace.lints.rust]\nunsafe_code = \"forbid\"\n";
const INHERIT: &str = "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n";
const EXEMPT: &str = "[package]\nname = \"ucdk-capi\"\n\n[lints.rust]\nunsafe_code = \"deny\"\n";

fn project(
    name: &str,
    root_manifest: &str,
    crates: &[(&str, &str)],
) -> (std::path::PathBuf, Vec<String>) {
    let root = scratch(name);
    write(&root, "Cargo.toml", root_manifest);
    write(&root, "xtask/Cargo.toml", INHERIT);
    let mut names = vec!["xtask".to_owned()];
    for (crate_name, manifest) in crates {
        write(&root, &format!("crates/{crate_name}/Cargo.toml"), manifest);
        names.push((*crate_name).to_owned());
    }
    (root, names)
}

fn rules(name: &str, root_manifest: &str, crates: &[(&str, &str)]) -> Vec<(&'static str, String)> {
    let (root, names) = project(name, root_manifest, crates);
    check_unsafe_scope(&root, &names)
        .into_iter()
        .map(|f| (f.rule, f.subject))
        .collect()
}

/// Proves: a workspace that forbids `unsafe`, whose members inherit that and whose C ABI crate
/// lowers it to `deny`, passes. Catches a gate that refuses the intended shape.
#[test]
fn the_intended_shape_passes() {
    let found = rules(
        "unsafe-ok",
        FORBID,
        &[("a", INHERIT), ("ucdk-capi", EXEMPT)],
    );
    assert!(found.is_empty(), "{found:?}");
}

/// Proves: the repository itself passes. Catches a manifest that drifts from the rule.
#[test]
fn this_repository_passes() {
    let root = workspace_root();
    let names = members(&root).expect("members");
    let found = check_unsafe_scope(&root, &names);
    assert!(found.is_empty(), "{found:?}");
}

/// Proves: a second crate that sets `unsafe_code` is refused, however it sets it. Catches an
/// exemption that spreads past the C ABI crate.
#[test]
fn another_crate_may_not_set_the_lint() {
    for setting in ["allow", "deny", "forbid"] {
        let other = format!("[lints.rust]\nunsafe_code = \"{setting}\"\n");
        let found = rules("unsafe-other", FORBID, &[("a", &other)]);
        assert_eq!(
            found,
            [("unsafe-override", "a/Cargo.toml".to_owned())],
            "{setting}"
        );
    }
}

/// Proves: a crate that does not inherit the workspace lints is refused, since it would escape
/// the `forbid`. Catches a manifest with no `[lints]` table, or one without `workspace = true`.
#[test]
fn a_crate_that_does_not_inherit_the_lints_is_refused() {
    for manifest in [
        "[package]\nname = \"a\"\n",
        "[lints]\n# workspace = true\n",
        "[dependencies]\nworkspace = true\n",
    ] {
        let found = rules("unsafe-inherit", FORBID, &[("a", manifest)]);
        assert_eq!(
            found,
            [("lints-not-inherited", "a/Cargo.toml".to_owned())],
            "{manifest}"
        );
    }
}

/// Proves: the exempt crate must say `deny`; `allow`, `forbid` or nothing is refused. Catches an
/// exemption widened to `allow` or dropped.
#[test]
fn the_exempt_crate_must_deny() {
    for setting in ["allow", "warn", "forbid"] {
        let manifest = format!("[lints.rust]\nunsafe_code = \"{setting}\"\n");
        let found = rules("unsafe-exempt", FORBID, &[("ucdk-capi", &manifest)]);
        assert_eq!(
            found,
            [(
                "unsafe-exemption-not-deny",
                "ucdk-capi/Cargo.toml".to_owned()
            )]
        );
    }
    let found = rules("unsafe-exempt-none", FORBID, &[("ucdk-capi", INHERIT)]);
    assert_eq!(
        found,
        [(
            "unsafe-exemption-not-deny",
            "ucdk-capi/Cargo.toml".to_owned()
        )]
    );
}

/// Proves: a root manifest that stops forbidding, or is missing, is refused, and so is a member
/// with no manifest. Catches a workspace that quietly allows `unsafe`.
#[test]
fn the_root_must_forbid_and_every_manifest_must_exist() {
    let allow = "[workspace.lints.rust]\nunsafe_code = \"allow\"\n";
    assert_eq!(
        rules("unsafe-root-allow", allow, &[]),
        [("unsafe-not-forbidden", "Cargo.toml".to_owned())]
    );
    let (root, names) = project("unsafe-root-missing", FORBID, &[]);
    std::fs::remove_file(root.join("Cargo.toml")).expect("remove");
    std::fs::remove_file(root.join("xtask/Cargo.toml")).expect("remove");
    let found: Vec<_> = check_unsafe_scope(&root, &names)
        .into_iter()
        .map(|f| (f.rule, f.subject))
        .collect();
    assert_eq!(
        found,
        [
            ("unsafe-unreadable", "Cargo.toml".to_owned()),
            ("unsafe-unreadable", "xtask/Cargo.toml".to_owned()),
        ]
    );
}
