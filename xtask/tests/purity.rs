// SPDX-License-Identifier: Apache-2.0
//! The purity template check: every crate marked pure carries the deny-list `clippy.toml`.

mod common;

use std::fs;

use common::{scratch, write};
use xtask::coverage::members;
use xtask::purity::{ALLOWANCES, REQUIRED, check_purity, is_pure, missing};
use xtask::workspace_root;

const PURE: &str = "[package]\nname = \"p\"\n\n[package.metadata.ucdk]\npure = true\n";

/// The template the repository's own pure crates carry.
fn template() -> String {
    fs::read_to_string(workspace_root().join("crates/ucdk-core/clippy.toml"))
        .expect("the core crate carries the template")
}

fn project(name: &str, manifest: &str, clippy: Option<&str>) -> (std::path::PathBuf, Vec<String>) {
    let root = scratch(name);
    write(&root, "crates/p/Cargo.toml", manifest);
    if let Some(text) = clippy {
        write(&root, "crates/p/clippy.toml", text);
    }
    (root, vec!["p".to_owned(), "xtask".to_owned()])
}

#[test]
fn purity_template_missing_fails() {
    let (root, names) = project("purity-missing", PURE, None);
    let findings = check_purity(&root, &names);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "purity-template-missing");
    assert_eq!(findings[0].subject, "p/clippy.toml");
}

#[test]
fn a_template_that_lacks_an_entry_names_what_it_lacks() {
    let without = template().replace("std::collections::HashMap", "std::collections::BTreeMap");
    let (root, names) = project("purity-incomplete", PURE, Some(&without));
    let findings = check_purity(&root, &names);
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "purity-template-incomplete");
    assert!(
        findings[0]
            .detail
            .contains("disallowed-types: std::collections::HashMap"),
        "{}",
        findings[0].detail
    );
}

#[test]
fn every_entry_is_required_in_its_own_setting() {
    let full = template();
    assert!(missing(&full).is_empty(), "{:?}", missing(&full));
    // An entry that is present but under the wrong setting does not count.
    let moved = full.replace("disallowed-macros = [", "other-setting = [");
    assert_eq!(missing(&moved).len(), REQUIRED[2].1.len());
    // Dropping the test allowances is caught: a crate-level file replaces the workspace one.
    let no_allowances: String = full
        .lines()
        .filter(|l| !l.starts_with("allow-"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(missing(&no_allowances), ALLOWANCES);
    assert_eq!(
        missing("").len(),
        ALLOWANCES.len() + REQUIRED.iter().map(|r| r.1.len()).sum::<usize>()
    );
}

#[test]
fn a_list_written_on_one_line_is_read_to_its_end() {
    let inline = "allow-unwrap-in-tests = true\nallow-expect-in-tests = true\n\
        disallowed-macros = [{ path = \"std::println\" }, { path = \"std::eprintln\" }, \
        { path = \"std::print\" }, { path = \"std::eprint\" }]\n";
    let lacking = missing(inline);
    assert!(
        lacking.iter().all(|l| !l.starts_with("disallowed-macros")),
        "{lacking:?}"
    );
}

#[test]
fn a_complete_template_passes_and_an_unmarked_crate_is_not_checked() {
    let (root, names) = project("purity-ok", PURE, Some(&template()));
    assert_eq!(check_purity(&root, &names), []);
    for (name, manifest) in [
        ("purity-unmarked", "[package]\nname = \"p\"\n"),
        ("purity-false", "[package.metadata.ucdk]\npure = false\n"),
        ("purity-other-section", "[package]\npure = true\n"),
    ] {
        let (root, names) = project(name, manifest, None);
        assert_eq!(check_purity(&root, &names), [], "{name}");
    }
    // A crate with no manifest at all is not marked either.
    let root = scratch("purity-no-manifest");
    assert_eq!(check_purity(&root, &["gone".to_owned()]), []);
}

#[test]
fn the_mark_is_read_from_its_own_section_only() {
    assert!(is_pure(PURE));
    assert!(is_pure("[package.metadata.ucdk]\npure = true # no I/O\n"));
    assert!(is_pure(
        "[package.metadata.ucdk]\nother = 1\npure = true\n[dependencies]\n"
    ));
    assert!(!is_pure(
        "[package.metadata.ucdk]\n[dependencies]\npure = true\n"
    ));
    assert!(!is_pure("[package.metadata.other]\npure = true\n"));
    assert!(!is_pure("pure = true\n"));
    assert!(!is_pure(""));
}

#[test]
fn the_repositorys_pure_crates_carry_the_template() {
    let root = workspace_root();
    let names = members(&root).expect("members");
    assert_eq!(check_purity(&root, &names), []);
    let marked: Vec<&String> = names
        .iter()
        .filter(|n| {
            fs::read_to_string(root.join("crates").join(n).join("Cargo.toml"))
                .is_ok_and(|m| is_pure(&m))
        })
        .collect();
    assert_eq!(marked, ["ucdk-core", "ucdk-release", "ucdk-release-proto"]);
}
