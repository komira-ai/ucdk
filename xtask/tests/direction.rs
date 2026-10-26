// SPDX-License-Identifier: Apache-2.0
//! The dependency-direction check: lower crates never depend on higher ones, and pure crates
//! never depend on impure ones.

mod common;

use common::{scratch, write};
use xtask::coverage::members;
use xtask::direction::{check_direction, dependencies};
use xtask::workspace_root;

const PURE: &str = "[package]\nname = \"x\"\n\n[package.metadata.ucdk]\npure = true\n";
const IMPURE: &str = "[package]\nname = \"x\"\n";

fn project(name: &str, crates: &[(&str, &str)]) -> (std::path::PathBuf, Vec<String>) {
    let root = scratch(name);
    let mut names = vec!["xtask".to_owned()];
    for (crate_name, manifest) in crates {
        write(&root, &format!("crates/{crate_name}/Cargo.toml"), manifest);
        names.push((*crate_name).to_owned());
    }
    (root, names)
}

fn with_deps(base: &str, section: &str, deps: &str) -> String {
    format!("{base}\n[{section}]\n{deps}\n")
}

#[test]
fn dependency_direction_violation_fails() {
    let core = with_deps(
        IMPURE,
        "dependencies",
        "ucdk-ledger = { path = \"../ucdk-ledger\" }",
    );
    let (root, names) = project(
        "direction-layer",
        &[("ucdk-core", &core), ("ucdk-ledger", IMPURE)],
    );
    let findings = check_direction(&root, &names);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "dependency-direction");
    assert_eq!(findings[0].subject, "ucdk-core/Cargo.toml");
    assert!(
        findings[0].detail.contains("ucdk-ledger"),
        "{}",
        findings[0].detail
    );
}

#[test]
fn a_lower_crate_may_not_reach_a_higher_one_through_any_section() {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let core = with_deps(IMPURE, section, "ucdk-ledger.workspace = true");
        let (root, names) = project(
            "direction-sections",
            &[("ucdk-core", &core), ("ucdk-ledger", IMPURE)],
        );
        let findings = check_direction(&root, &names);
        assert_eq!(findings.len(), 1, "{section}: {findings:?}");
        assert_eq!(findings[0].rule, "dependency-direction", "{section}");
    }
    let table = format!("{IMPURE}\n[dev-dependencies.ucdk-ledger]\npath = \"../ucdk-ledger\"\n");
    let (root, names) = project(
        "direction-table",
        &[("ucdk-core", &table), ("ucdk-ledger", IMPURE)],
    );
    assert_eq!(check_direction(&root, &names).len(), 1);
}

#[test]
fn a_pure_crate_depending_on_an_impure_one_fails() {
    let deps = "ucdk-ledger = { path = \"../ucdk-ledger\" }\nsha2.workspace = true";
    let (root, names) = project(
        "direction-pure",
        &[
            ("some-pure", &with_deps(PURE, "dependencies", deps)),
            ("ucdk-ledger", IMPURE),
        ],
    );
    let findings = check_direction(&root, &names);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "pure-depends-on-impure");
    assert_eq!(findings[0].subject, "some-pure/Cargo.toml");
}

#[test]
fn a_pure_crate_may_depend_on_pure_crates_and_outside_ones() {
    let deps = "# a comment\nother-pure = { path = \"../other-pure\" }\nsha2.workspace = true";
    let (root, names) = project(
        "direction-clean",
        &[
            ("some-pure", &with_deps(PURE, "dependencies", deps)),
            ("other-pure", PURE),
            (
                "ucdk-ledger",
                &with_deps(IMPURE, "dependencies", "other-pure.workspace = true"),
            ),
        ],
    );
    assert_eq!(check_direction(&root, &names), []);
}

#[test]
fn a_missing_manifest_is_skipped() {
    let (root, mut names) = project("direction-missing", &[]);
    names.push("not-there".to_owned());
    assert_eq!(check_direction(&root, &names), []);
}

#[test]
fn dependencies_are_read_from_every_form() {
    let manifest = "[package]\nname = \"p\"\n\n[dependencies]\na = \"1\"\nb.workspace = true\n\
        # c = \"1\"\n\n[dependencies.d]\npath = \"../d\"\n\n[features]\ne = []\n\n\
        [build-dependencies]\nf = \"1\"\n";
    let found: Vec<String> = dependencies(manifest).into_iter().collect();
    assert_eq!(found, ["a", "b", "d", "f"]);
}

#[test]
fn the_repository_follows_the_direction_rules() {
    let root = workspace_root();
    let names = members(&root).expect("members");
    assert_eq!(check_direction(&root, &names), []);
}

#[test]
fn direction_sees_target_specific_dependencies() {
    // Every plain, development and build section, under a cfg expression (which holds dots and
    // spaces) and under a bare target triple, in the list form and the table form.
    let manifest = "[package]\nname = \"p\"\n\n\
        [target.'cfg(target_os = \"linux\")'.dependencies]\na = \"1\"\nb.workspace = true\n\n\
        [target.\"cfg(any(unix, windows))\".dev-dependencies]\nc = \"1\"\n\n\
        [target.x86_64-unknown-linux-gnu.build-dependencies]\nd = \"1\"\n\n\
        [target.'cfg(unix)'.dependencies.e]\npath = \"../e\"\n\n\
        [target.'cfg(unix)'.features]\nnot-a-dependency = []\n\n\
        [target.broken]\nnor-this = \"1\"\n";
    let found: Vec<String> = dependencies(manifest).into_iter().collect();
    assert_eq!(found, ["a", "b", "c", "d", "e"]);

    // A lower crate cannot reach a higher one through a target-specific section.
    for section in [
        "target.'cfg(unix)'.dependencies",
        "target.'cfg(windows)'.dev-dependencies",
        "target.aarch64-apple-darwin.build-dependencies",
    ] {
        let core = with_deps(IMPURE, section, "ucdk-ledger.workspace = true");
        let (root, names) = project(
            "direction-target",
            &[("ucdk-core", &core), ("ucdk-ledger", IMPURE)],
        );
        let findings = check_direction(&root, &names);
        assert_eq!(findings.len(), 1, "{section}: {findings:?}");
        assert_eq!(findings[0].rule, "dependency-direction", "{section}");
    }

    // A pure crate cannot reach an impure one through one either.
    let deps = "ucdk-ledger = { path = \"../ucdk-ledger\" }";
    let pure = with_deps(PURE, "target.'cfg(unix)'.dependencies", deps);
    let (root, names) = project(
        "direction-target-pure",
        &[("some-pure", &pure), ("ucdk-ledger", IMPURE)],
    );
    let findings = check_direction(&root, &names);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "pure-depends-on-impure");
}

#[test]
fn direction_resolves_renamed_dependencies() {
    // The inline form, the dotted form, the table form, and a key that only looks like one.
    let manifest = "[package]\nname = \"p\"\n\n[dependencies]\n\
        core = { package = \"ucdk-core\", path = \"../ucdk-core\" }\n\
        ledger = { path = \"../ucdk-ledger\", package = 'ucdk-ledger' }\n\
        dotted.package = \"ucdk-api\"\n\
        dotted.workspace = true\n\
        plain = { version = \"1\", features = [\"package-like\"] }\n\
        text = { description = \"not a package = \\\"x\\\"\" }\n\
        spaced = {package=\"spaced-real\"}\n\
        odd = { package = 5 }\n\n\
        [dev-dependencies.alias]\npackage = \"ucdk-cli\"\npath = \"../x\"\n\n\
        [dev-dependencies.unrenamed]\npath = \"../y\"\n";
    let found: Vec<String> = dependencies(manifest).into_iter().collect();
    assert_eq!(
        found,
        [
            "odd",
            "plain",
            "spaced-real",
            "text",
            "ucdk-api",
            "ucdk-cli",
            "ucdk-core",
            "ucdk-ledger",
            "unrenamed"
        ]
    );

    // A lower crate cannot hide a dependency behind a rename.
    let core = with_deps(
        IMPURE,
        "dependencies",
        "up = { package = \"ucdk-ledger\", path = \"../ucdk-ledger\" }",
    );
    let (root, names) = project(
        "direction-renamed",
        &[("ucdk-core", &core), ("ucdk-ledger", IMPURE)],
    );
    let findings = check_direction(&root, &names);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].detail.contains("ucdk-ledger"));
}
