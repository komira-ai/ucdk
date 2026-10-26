// SPDX-License-Identifier: Apache-2.0
//! The generated SDK sources: written by `gen-sdk`, held to the generator by the `sdk-fresh`
//! gate, and the SDK's tests run by `sdk-test`.

mod common;

use std::fs;

use common::{Fake, cache_tools, install_protos, install_sdk, scratch, strings, write};
use xtask::cli;
use xtask::sdk::{check_fresh, test, write as generate_into};

const WIRE: &str = "sdk/python/ucdk/generated/_wire.py";
const WIRE_TS: &str = "sdk/typescript/src/generated/wire.ts";
/// The files both generators write: six Python, five TypeScript.
const FILES: usize = 11;

#[test]
fn the_committed_sdk_is_what_the_generator_writes() {
    let (findings, files) = check_fresh(&xtask::workspace_root());
    assert!(findings.is_empty(), "{findings:?}");
    assert_eq!(files, FILES);
}

#[test]
fn a_written_sdk_passes_and_the_gate_names_each_way_it_can_differ() {
    let root = scratch("sdk-fresh");
    install_sdk(&root);
    assert_eq!(check_fresh(&root), (Vec::new(), FILES));

    // A generated file edited by hand.
    let wire = root.join(WIRE);
    let text = fs::read_to_string(&wire).expect("_wire.py");
    fs::write(&wire, format!("{text}# edited\n")).expect("edit");
    let (findings, _) = check_fresh(&root);
    assert_eq!(
        findings
            .iter()
            .map(|f| (f.rule, f.subject.as_str()))
            .collect::<Vec<_>>(),
        [("sdk-stale", WIRE)]
    );

    // One that is not committed.
    fs::remove_file(&wire).expect("remove");
    let (findings, _) = check_fresh(&root);
    assert_eq!(
        findings
            .iter()
            .map(|f| (f.rule, f.subject.as_str()))
            .collect::<Vec<_>>(),
        [("sdk-missing", WIRE)]
    );

    // One that the generator no longer writes; a file that is not Python is left alone.
    generate_into(&root).expect("rewrite");
    write(&root, "sdk/python/ucdk/generated/old.py", "x = 1\n");
    write(&root, "sdk/python/ucdk/generated/notes.txt", "kept\n");
    let (findings, _) = check_fresh(&root);
    assert_eq!(
        findings
            .iter()
            .map(|f| (f.rule, f.subject.as_str()))
            .collect::<Vec<_>>(),
        [("sdk-extra", "sdk/python/ucdk/generated/old.py")]
    );
    assert!(findings[0].detail.contains("gen-sdk"));

    // Writing again removes it.
    let written = generate_into(&root).expect("write");
    assert_eq!(written.len(), FILES);
    assert!(!root.join("sdk/python/ucdk/generated/old.py").exists());
    assert!(root.join("sdk/python/ucdk/generated/notes.txt").exists());
    assert_eq!(check_fresh(&root), (Vec::new(), FILES));
}

#[test]
fn the_gate_holds_the_typescript_files_too() {
    let root = scratch("sdk-fresh-ts");
    install_sdk(&root);
    let subjects = |root: &std::path::Path| -> Vec<(&'static str, String)> {
        check_fresh(root)
            .0
            .iter()
            .map(|f| (f.rule, f.subject.clone()))
            .collect()
    };

    let wire = root.join(WIRE_TS);
    let text = fs::read_to_string(&wire).expect("wire.ts");
    fs::write(&wire, format!("{text}// edited\n")).expect("edit");
    assert_eq!(subjects(&root), [("sdk-stale", WIRE_TS.to_owned())]);

    fs::remove_file(&wire).expect("remove");
    assert_eq!(subjects(&root), [("sdk-missing", WIRE_TS.to_owned())]);

    // A TypeScript file the generator no longer writes; Python files and others are not its.
    generate_into(&root).expect("rewrite");
    write(&root, "sdk/typescript/src/generated/old.ts", "export {};\n");
    write(&root, "sdk/typescript/src/generated/notes.py", "kept\n");
    assert_eq!(
        subjects(&root),
        [(
            "sdk-extra",
            "sdk/typescript/src/generated/old.ts".to_owned()
        )]
    );
    generate_into(&root).expect("rewrite");
    assert!(!root.join("sdk/typescript/src/generated/old.ts").exists());
    assert!(root.join("sdk/typescript/src/generated/notes.py").exists());
    assert_eq!(check_fresh(&root), (Vec::new(), FILES));
}

#[test]
fn an_sdk_that_was_never_generated_is_missing_every_file() {
    let root = scratch("sdk-never");
    install_protos(&root);
    let (findings, files) = check_fresh(&root);
    assert_eq!(files, FILES);
    assert_eq!(findings.len(), FILES);
    assert!(findings.iter().all(|f| f.rule == "sdk-missing"));
}

#[test]
fn a_tree_without_the_protos_has_nothing_to_generate_from() {
    let root = scratch("sdk-no-protos");
    let (findings, files) = check_fresh(&root);
    assert_eq!(files, 0);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule, "sdk-generate");
    assert!(findings[0].detail.contains("do not compile"));
    assert!(generate_into(&root).is_err());
}

#[test]
fn an_sdk_directory_that_cannot_be_written_is_an_error() {
    let root = scratch("sdk-unwritable");
    install_sdk(&root);
    // A file where the generated directory has to be.
    fs::remove_dir_all(root.join("sdk")).expect("remove");
    write(&root, "sdk", "a file\n");
    assert!(generate_into(&root).is_err());
}

#[test]
fn a_generated_file_that_cannot_be_written_or_removed_is_an_error() {
    let root = scratch("sdk-unwritable-file");
    install_sdk(&root);
    // A directory named like a generated file.
    fs::remove_file(root.join(WIRE)).expect("remove");
    fs::create_dir(root.join(WIRE)).expect("a directory");
    assert!(generate_into(&root).is_err());
    // A directory named like a stale `.py` file.
    fs::remove_dir(root.join(WIRE)).expect("remove");
    fs::create_dir(root.join("sdk/python/ucdk/generated/stale.py")).expect("a directory");
    assert!(generate_into(&root).is_err());
}

#[test]
fn the_gen_sdk_verb_writes_and_lists_the_files() {
    let root = scratch("sdk-verb");
    install_sdk(&root);
    fs::remove_file(root.join(WIRE)).expect("remove");
    let (code, text) = cli::run(&strings(&["gen-sdk"]), &|_| None, &Fake::default(), &root);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains(&format!("wrote {WIRE}\n")), "{text}");
    assert_eq!(text.lines().count(), FILES);
    assert!(root.join(WIRE).is_file());

    let (code, text) = cli::run(
        &strings(&["gen-sdk"]),
        &|_| None,
        &Fake::default(),
        &scratch("sdk-verb-empty"),
    );
    assert_eq!(code, 1);
    assert!(text.contains("do not compile"), "{text}");
}

/// A checkout with test files for both SDKs, and the pinned Node installed.
fn with_tests(name: &str) -> std::path::PathBuf {
    let root = scratch(name);
    write(&root, "sdk/python/tests/test_a.py", "");
    write(&root, "sdk/python/tests/helper.py", "");
    write(&root, "sdk/typescript/tests/a.test.ts", "");
    write(&root, "sdk/typescript/tests/b.test.ts", "");
    write(&root, "sdk/typescript/tests/support.ts", "");
    cache_tools(&root);
    root
}

#[test]
fn sdk_test_runs_the_python_tests_then_the_typescript_tests_in_their_directories() {
    let root = with_tests("sdk-test");
    let fake = Fake::default();
    let (findings, examined) = test(&root, &fake);
    assert!(findings.is_empty(), "{findings:?}");
    assert_eq!(
        examined,
        "1 Python test file and 2 TypeScript test files, against the library built from ucdk-capi"
    );
    // The library is built first, and the tests run in the SDK directories.
    assert_eq!(
        *fake.log.borrow(),
        [
            "cargo build --locked -p ucdk-capi --features test-clock",
            "python3 -B -m unittest discover -s tests -t .",
            "node --test --test-timeout=60000 tests/a.test.ts tests/b.test.ts"
        ]
    );
}

#[test]
fn a_failing_suite_is_a_finding_and_the_next_one_does_not_run() {
    let root = with_tests("sdk-test-fail");
    let failing = Fake {
        fail_on: vec!["unittest"],
        ..Fake::default()
    };
    let (findings, _) = test(&root, &failing);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule, "sdk-test");
    assert!(!failing.log.borrow().iter().any(|l| l.starts_with("node")));

    let failing = Fake {
        fail_on: vec!["node --test"],
        ..Fake::default()
    };
    let (findings, _) = test(&root, &failing);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule, "sdk-test");
    assert!(findings[0].subject.starts_with("node --test"));

    // A library that does not build is not followed by tests that cannot load it.
    let unbuilt = Fake {
        fail_on: vec!["cargo build"],
        ..Fake::default()
    };
    let (findings, _) = test(&root, &unbuilt);
    assert_eq!(findings.len(), 1);
    assert!(findings[0].subject.starts_with("cargo build"));
    assert_eq!(
        *unbuilt.log.borrow(),
        ["cargo build --locked -p ucdk-capi --features test-clock"]
    );
}

#[test]
fn node_is_the_pinned_one_and_a_node_that_cannot_be_fetched_is_a_finding() {
    let root = with_tests("sdk-test-node");
    let fake = Fake::default();
    test(&root, &fake);
    // The tests run on the Node under `target/xtask-tools`, not the machine's.
    let node = std::rc::Rc::new(std::cell::RefCell::new(None));
    let spy = Fake {
        on_run: Some(Box::new({
            let node = std::rc::Rc::clone(&node);
            move |cmd| {
                if cmd.program == "node" {
                    *node.borrow_mut() = cmd.path_prepend.clone();
                }
            }
        })),
        ..Fake::default()
    };
    test(&root, &spy);
    assert_eq!(node.take(), Some(xtask::fetch::bin_dir(&root)));

    // Not installed, and the download fails: nothing runs on whatever Node is on the path.
    let bare = scratch("sdk-test-no-node");
    write(&bare, "sdk/typescript/tests/a.test.ts", "");
    let failing = Fake {
        fail_on: vec!["curl"],
        ..Fake::default()
    };
    let (findings, _) = test(&bare, &failing);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].rule, "fetch");
    assert_eq!(findings[0].subject, "node");
    assert!(!failing.log.borrow().iter().any(|l| l.starts_with("node")));
}

#[test]
fn the_sdk_test_verb_reports_a_verdict() {
    let root = with_tests("sdk-test-verb");
    let (code, text) = cli::run(&strings(&["sdk-test"]), &|_| None, &Fake::default(), &root);
    assert_eq!(
        (code, text.as_str()),
        (
            0,
            "== sdk-test: ok (1 Python test file and 2 TypeScript test files, against the library built from ucdk-capi)\n"
        )
    );
    let failing = Fake {
        fail_on: vec!["unittest"],
        ..Fake::default()
    };
    let (code, text) = cli::run(&strings(&["sdk-test"]), &|_| None, &failing, &root);
    assert_eq!(code, 1);
    assert!(
        text.starts_with("== sdk-test: FAILED (1 Python test file"),
        "{text}"
    );
    assert!(text.contains("[sdk-test] python3 -B -m unittest"), "{text}");

    // A checkout with no tests directory has nothing to run, and says so.
    let empty = scratch("sdk-test-none");
    cache_tools(&empty);
    let (_, text) = cli::run(&strings(&["sdk-test"]), &|_| None, &Fake::default(), &empty);
    assert!(
        text.contains("0 Python test files and 0 TypeScript test files"),
        "{text}"
    );
}
