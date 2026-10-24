// SPDX-License-Identifier: Apache-2.0
//! The coverage gate, fed fixture lcov through the same code path the CI job uses.

mod common;

use std::collections::BTreeMap;

use common::{Fake, cache_tools, scratch, strings, write};
use xtask::Finding;
use xtask::coverage::{
    Counts, FLOOR, LCOV, allow_generated_only, check_branches_measured, check_main_rs, crate_of,
    excluded, gate, generated_only, members, parse_lcov, run,
};

/// One lcov record: `lines` lines of which `hit` ran, `branches` branches of which `taken` ran.
fn record(path: &str, lines: u64, hit: u64, branches: u64, taken: u64) -> String {
    let mut out = format!("SF:{path}\n");
    for n in 0..lines {
        out.push_str(&format!("DA:{},{}\n", n + 1, u64::from(n < hit)));
    }
    for n in 0..branches {
        let count = if n < taken {
            "3"
        } else if n % 2 == 0 {
            "0"
        } else {
            "-"
        };
        out.push_str(&format!("BRDA:1,0,{n},{count}\n"));
    }
    out.push_str("end_of_record\n");
    out
}

fn findings(lcov: &str, members: &[&str]) -> Vec<(&'static str, String)> {
    let counts = parse_lcov(lcov, "/r");
    gate(&counts, &strings(members), FLOOR)
        .0
        .into_iter()
        .map(|f| (f.rule, f.subject))
        .collect()
}

#[test]
fn coverage_below_floor_fails() {
    let at_floor = record("/r/crates/a/src/lib.rs", 100, 99, 0, 0);
    assert!(findings(&at_floor, &["a"]).is_empty());
    let below = record("/r/crates/a/src/lib.rs", 100, 98, 0, 0);
    assert_eq!(
        findings(&below, &["a"]),
        [("coverage-lines", "a".to_owned())]
    );
}

#[test]
fn coverage_checks_each_crate_separately() {
    // Together the two crates sit at 99.5%, above the floor; the thin one still fails.
    let lcov = [
        record("/r/crates/fat/src/lib.rs", 10_000, 10_000, 0, 0),
        record("/r/crates/thin/src/lib.rs", 100, 50, 0, 0),
    ]
    .concat();
    assert_eq!(
        findings(&lcov, &["fat", "thin"]),
        [("coverage-lines", "thin".to_owned())]
    );
}

#[test]
fn coverage_branch_below_floor_fails() {
    let ok = record("/r/xtask/src/lib.rs", 10, 10, 100, 99);
    assert!(findings(&ok, &["xtask"]).is_empty());
    let low = record("/r/xtask/src/lib.rs", 10, 10, 100, 98);
    assert_eq!(
        findings(&low, &["xtask"]),
        [("coverage-branches", "xtask".to_owned())]
    );
}

#[test]
fn coverage_zero_measured_lines_fails() {
    // `empty` has no record at all; `stub` has only a main.rs, which is left out.
    let lcov = [
        record("/r/crates/ok/src/lib.rs", 10, 10, 0, 0),
        record("/r/crates/stub/src/main.rs", 3, 3, 0, 0),
    ]
    .concat();
    assert_eq!(
        findings(&lcov, &["ok", "empty", "stub"]),
        [
            ("coverage-no-lines", "empty".to_owned()),
            ("coverage-no-lines", "stub".to_owned())
        ]
    );
}

#[test]
fn main_rs_over_three_lines_fails() {
    let root = scratch("main-rs");
    write(
        &root,
        "xtask/src/main.rs",
        "// SPDX-License-Identifier: Apache-2.0\nfn main() {\n    x();\n}\n",
    );
    write(
        &root,
        "crates/short/src/main.rs",
        "// SPDX-License-Identifier: Apache-2.0\nfn main() {\n    a();\n}\n",
    );
    write(
        &root,
        "crates/long/src/main.rs",
        "// SPDX-License-Identifier: Apache-2.0\nfn main() {\n    a();\n\n    b();\n    c();\n}\n",
    );
    write(&root, "crates/lib/src/lib.rs", "");
    let names = strings(&["lib", "long", "short", "xtask"]);
    let found: Vec<_> = check_main_rs(&root, &names)
        .into_iter()
        .map(|f| (f.rule, f.subject))
        .collect();
    assert_eq!(found, [("main-rs-too-long", "long/src/main.rs".to_owned())]);
}

#[test]
fn coverage_leaves_out_generated_code_and_main_rs_only() {
    let lcov = [
        record("/r/crates/a/src/lib.rs", 10, 10, 2, 2),
        record("/r/target/debug/build/a/out/gen.rs", 50, 0, 10, 0),
        record("/r/crates/a/src/main.rs", 5, 0, 0, 0),
        record("/r/crates/a/src/main_helper.rs", 5, 5, 0, 0),
    ]
    .concat();
    let counts = parse_lcov(&lcov, "/r");
    let a = Counts {
        lines: 15,
        lines_hit: 15,
        branches: 2,
        branches_hit: 2,
    };
    assert_eq!(counts, BTreeMap::from([("a".to_owned(), a)]));
}

#[test]
fn crate_of_names_the_directory_under_crates_or_the_first_directory() {
    assert_eq!(crate_of("/r/crates/ucdk-cli/src/lib.rs", "/r"), "ucdk-cli");
    assert_eq!(crate_of("/r/xtask/src/lib.rs", "/r"), "xtask");
    assert_eq!(crate_of("crates/a/src/lib.rs", "/r"), "a");
    assert_eq!(crate_of("/elsewhere/x.rs", "/r"), "elsewhere");
    assert_eq!(crate_of("", "/r"), "");
}

#[test]
fn members_are_the_crate_directories_and_xtask() {
    let root = scratch("members");
    write(&root, "crates/b/Cargo.toml", "");
    write(&root, "crates/a/Cargo.toml", "");
    write(&root, "crates/not-a-crate.txt", "");
    assert_eq!(members(&root).expect("members"), ["a", "b", "xtask"]);
    assert!(members(&scratch("members-none")).is_err());
}

#[test]
fn the_report_has_one_line_per_crate_with_percentages() {
    let lcov = record("/r/crates/a/src/lib.rs", 3, 2, 4, 1);
    let (_, report) = gate(&parse_lcov(&lcov, "/r"), &strings(&["a", "z"]), FLOOR);
    assert_eq!(
        report,
        [
            "a: lines 66.66% (2/3), branches 25.00% (1/4)",
            "z: lines 0.00% (0/0), branches 0.00% (0/0)"
        ]
    );
}

fn project(name: &str) -> std::path::PathBuf {
    let root = scratch(name);
    write(&root, "crates/c/src/lib.rs", "");
    write(&root, "xtask/src/lib.rs", "");
    cache_tools(&root);
    root
}

fn llvm_cov_writes(lcov: &'static str) -> Option<common::Hook> {
    Some(Box::new(move |cmd| {
        if cmd.args.iter().any(|a| a == "llvm-cov") {
            let root = cmd.dir.canonicalize().expect("root");
            let text = lcov.replace("/r/", &format!("{}/", root.display()));
            write(&cmd.dir, LCOV, &text);
        }
    }))
}

#[test]
fn the_verb_runs_the_tests_on_the_pinned_nightly_and_gates_the_lcov_file() {
    let good = [
        record("/r/crates/c/src/lib.rs", 10, 10, 2, 2),
        record("/r/xtask/src/lib.rs", 10, 10, 0, 0),
    ]
    .concat();
    let root = project("verb-good");
    let fake = Fake {
        on_run: llvm_cov_writes(Box::leak(good.into_boxed_str())),
        ..Fake::default()
    };
    let (found, report) = run(&root, &fake);
    assert!(found.is_empty(), "{found:?}");
    assert_eq!(report.len(), 2);
    let log = fake.log.borrow().join("\n");
    assert!(log.contains("rustup toolchain install nightly-2026-10-01 --profile minimal --component llvm-tools-preview"), "{log}");
    assert!(
        log.contains(
            "cargo +nightly-2026-10-01 llvm-cov --branch --workspace --locked --lcov --output-path"
        ),
        "{log}"
    );

    let bad = [
        record("/r/crates/c/src/lib.rs", 10, 5, 2, 2),
        record("/r/xtask/src/lib.rs", 10, 10, 0, 0),
    ]
    .concat();
    let fake = Fake {
        on_run: llvm_cov_writes(Box::leak(bad.into_boxed_str())),
        ..Fake::default()
    };
    let (found, _) = run(&project("verb-bad"), &fake);
    assert_eq!(
        found
            .iter()
            .map(|f| (f.rule, f.subject.as_str()))
            .collect::<Vec<_>>(),
        [("coverage-lines", "c")]
    );
}

#[test]
fn the_verb_reports_a_failed_step_or_a_missing_lcov_file() {
    let fake = Fake {
        fail_on: vec!["llvm-cov"],
        ..Fake::default()
    };
    let (found, report) = run(&project("verb-fail-run"), &fake);
    assert_eq!(
        found.iter().map(|f| f.rule).collect::<Vec<_>>(),
        ["coverage-run"]
    );
    assert!(report.is_empty());

    let (found, _) = run(&project("verb-no-lcov"), &Fake::default());
    assert_eq!(
        found.iter().map(|f| f.rule).collect::<Vec<_>>(),
        ["coverage-read"]
    );

    let blocked = project("verb-blocked");
    write(
        &blocked,
        "target/xtask-coverage",
        "a file where the directory should be",
    );
    let (found, _) = run(&blocked, &Fake::default());
    assert_eq!(
        found.iter().map(|f| f.rule).collect::<Vec<_>>(),
        ["coverage-run"]
    );

    let root = scratch("verb-no-crates");
    cache_tools(&root);
    let (found, _) = run(&root, &Fake::default());
    assert_eq!(
        found.iter().map(|f| f.rule).collect::<Vec<_>>(),
        ["coverage-read"]
    );
}

#[test]
fn the_verb_stops_when_a_tool_cannot_be_installed() {
    let fake = Fake {
        fail_on: vec!["curl"],
        ..Fake::default()
    };
    let (found, report) = run(&scratch("verb-no-tools"), &fake);
    assert_eq!(found.iter().map(|f| f.rule).collect::<Vec<_>>(), ["fetch"]);
    assert!(report.is_empty());
    assert!(
        fake.log
            .borrow()
            .iter()
            .all(|c| c.starts_with("curl") && c.contains("llvm-cov"))
    );
}

#[test]
fn only_prost_output_and_a_members_src_main_rs_are_left_out() {
    for path in [
        "target/debug/build/ucdk-proto-0a1b/out/ucdk.resource.v1.rs",
        "target/llvm-cov-target/debug/build/p-1/out/x.rs",
        "crates/ucdk-cli/src/main.rs",
        "xtask/src/main.rs",
    ] {
        assert!(excluded(path), "{path}");
    }
    for path in [
        "crates/a/src/bin/tool/main.rs",
        "crates/a/src/bin/main.rs",
        "crates/a/tests/main.rs",
        "crates/a/src/target/mod.rs",
        "crates/a/src/target/debug/build/x/out/y.rs",
        "crates/a/src/x/main.rs",
        "crates/a/b/src/main.rs",
        "src/main.rs",
        "target/debug/build/p-1/out/notes.txt",
        "target/debug/deps/x.rs",
        "xtask/tests/main.rs",
    ] {
        assert!(!excluded(path), "{path}");
    }
    // And the lcov parser measures what `excluded` keeps.
    let lcov = [
        record("/r/crates/a/src/bin/tool/main.rs", 4, 0, 0, 0),
        record("/r/crates/a/tests/main.rs", 4, 0, 0, 0),
        record("/r/crates/a/src/target/mod.rs", 4, 0, 0, 0),
    ]
    .concat();
    let counts = parse_lcov(&lcov, "/r");
    assert_eq!(counts["a"].lines, 12);
    assert_eq!(
        findings(&lcov, &["a"]),
        [("coverage-lines", "a".to_owned())]
    );
}

#[test]
fn coverage_with_no_branch_measured_anywhere_fails() {
    let none = [
        record("/r/crates/a/src/lib.rs", 10, 10, 0, 0),
        record("/r/xtask/src/lib.rs", 10, 10, 0, 0),
    ]
    .concat();
    let found = check_branches_measured(&parse_lcov(&none, "/r"));
    assert_eq!(
        found
            .iter()
            .map(|f| (f.rule, f.subject.as_str()))
            .collect::<Vec<_>>(),
        [("coverage-no-branches", "workspace")]
    );
    assert_eq!(check_branches_measured(&BTreeMap::new()).len(), 1);
    // One crate with branches is enough; a crate without any is fine on its own.
    let some = [
        record("/r/crates/a/src/lib.rs", 10, 10, 0, 0),
        record("/r/xtask/src/lib.rs", 10, 10, 4, 4),
    ]
    .concat();
    assert!(check_branches_measured(&parse_lcov(&some, "/r")).is_empty());
}

#[test]
fn the_verb_fails_when_the_lcov_file_has_no_branch_records() {
    let no_branches = [
        record("/r/crates/c/src/lib.rs", 10, 10, 0, 0),
        record("/r/xtask/src/lib.rs", 10, 10, 0, 0),
    ]
    .concat();
    let fake = Fake {
        on_run: llvm_cov_writes(Box::leak(no_branches.into_boxed_str())),
        ..Fake::default()
    };
    let (found, _) = run(&project("verb-no-branches"), &fake);
    assert_eq!(
        found.iter().map(|f| f.rule).collect::<Vec<_>>(),
        ["coverage-no-branches"]
    );
}

const HEADER: &str = "// SPDX-License-Identifier: Apache-2.0\n";
const GLUE: &str = "#[allow(clippy::all)]\npub mod v1 {\n    include!(concat!(env!(\"OUT_DIR\"), \"/x.rs\"));\n}\n";

fn generated(root: &std::path::Path, members: &[&str]) -> Vec<String> {
    generated_only(root, &strings(members))
        .into_iter()
        .collect()
}

#[test]
fn a_crate_of_only_include_glue_is_generated_only() {
    let root = scratch("generated-only-glue");
    write(&root, "crates/pb/src/lib.rs", &format!("{HEADER}{GLUE}"));
    write(
        &root,
        "crates/inner/src/lib.rs",
        &format!("{HEADER}#![allow(missing_docs)]\nmod gen {{\n    include!(\"x.rs\");\n}}\n"),
    );
    // A second file of glue under a subdirectory, and a file that is not Rust.
    write(
        &root,
        "crates/inner/src/more/mod.rs",
        "include!(\"y.rs\");\n",
    );
    write(&root, "crates/inner/src/notes.txt", "fn not_rust() {}\n");
    assert_eq!(generated(&root, &["inner", "pb"]), ["inner", "pb"]);
}

#[test]
fn a_crate_with_a_hand_written_line_is_not_generated_only() {
    let root = scratch("generated-only-code");
    // One line that is not glue, per crate; each near-miss of a glue shape is also code.
    let cases = [
        ("fn", "fn helper() {}"),
        ("attr-open", "#[derive(Debug"),
        ("inner-attr-open", "#![allow(x"),
        ("pub-mod-decl", "pub mod v1;"),
        ("mod-decl", "mod v1;"),
        ("include-open", "include!(x)"),
        ("use", "use std::fmt;"),
    ];
    for (name, code) in cases {
        write(
            &root,
            &format!("crates/{name}/src/lib.rs"),
            &format!("{GLUE}{code}\n"),
        );
    }
    // No `include!` at all: an empty crate is an ordinary crate, and is failed as one.
    write(&root, "crates/empty/src/lib.rs", HEADER);
    // No `src` directory at all.
    write(&root, "crates/nosrc/Cargo.toml", "");
    // Glue in one file does not excuse code in another.
    write(&root, "crates/mixed/src/lib.rs", GLUE);
    write(&root, "crates/mixed/src/util.rs", "pub fn f() {}\n");
    let names: Vec<&str> = cases
        .iter()
        .map(|(n, _)| *n)
        .chain(["empty", "nosrc", "mixed"])
        .collect();
    assert_eq!(generated(&root, &names), Vec::<String>::new());
}

#[test]
fn only_the_no_lines_finding_of_a_generated_only_crate_is_allowed() {
    let generated = std::collections::BTreeSet::from(["pb".to_owned()]);
    let mut found = vec![
        Finding::new("coverage-no-lines", "pb", "no line measured"),
        Finding::new("coverage-no-lines", "other", "no line measured"),
        Finding::new("coverage-lines", "pb", "below the floor"),
    ];
    allow_generated_only(&mut found, &generated);
    assert_eq!(
        found
            .iter()
            .map(|f| (f.rule, f.subject.as_str()))
            .collect::<Vec<_>>(),
        [("coverage-no-lines", "other"), ("coverage-lines", "pb")]
    );
}

#[test]
fn the_verb_passes_a_generated_only_crate_and_still_fails_an_empty_one() {
    let lcov = [
        record("/r/crates/c/src/lib.rs", 10, 10, 2, 2),
        record("/r/xtask/src/lib.rs", 10, 10, 0, 0),
    ]
    .concat();
    let lcov: &'static str = Box::leak(lcov.into_boxed_str());

    let root = project("verb-generated");
    write(&root, "crates/pb/src/lib.rs", GLUE);
    let fake = Fake {
        on_run: llvm_cov_writes(lcov),
        ..Fake::default()
    };
    let (found, report) = run(&root, &fake);
    assert!(found.is_empty(), "{found:?}");
    assert!(
        report
            .iter()
            .any(|l| l.starts_with("pb: lines 0.00% (0/0)"))
    );

    let root = project("verb-empty");
    write(&root, "crates/hollow/src/lib.rs", HEADER);
    let fake = Fake {
        on_run: llvm_cov_writes(lcov),
        ..Fake::default()
    };
    let (found, _) = run(&root, &fake);
    assert_eq!(
        found
            .iter()
            .map(|f| (f.rule, f.subject.as_str()))
            .collect::<Vec<_>>(),
        [("coverage-no-lines", "hollow")]
    );
}
