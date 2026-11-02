// SPDX-License-Identifier: Apache-2.0
//! `cargo xtask build-confirm`: the two release builds of the build tools, the verb that runs the
//! confirmation tests against them, and the workflow that runs the verb on request.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::{Fake, scratch, strings};
use xtask::cli;
use xtask::cmd::{Cmd, Runner, System};
use xtask::fetch::{
    BAZEL, BUCK2, Tool, bin_dir, confirmation_tools, ensure, ensure_all, ensure_only, sha256_hex,
};

const TEST_LINE: &str = "cargo test --locked -p ucdk-release-build --test confirm -- \
                         --include-ignored --nocapture --test-threads=1";

fn none(_: &str) -> Option<String> {
    None
}

fn read(path: &str) -> String {
    fs::read_to_string(xtask::workspace_root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn bin(dir: &Path) -> PathBuf {
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).expect("bin dir");
    bin
}

/// Marks both confirmation tools as installed under `root`, as an install leaves them.
fn cache_confirmation_tools(root: &Path) {
    let bin = bin_dir(root);
    fs::create_dir_all(&bin).expect("bin dir");
    for tool in confirmation_tools() {
        let body = format!("stand-in for {}", tool.name);
        fs::write(bin.join(&tool.name), &body).expect("binary");
        fs::write(
            bin.join(format!("{}.bin-sha256", tool.name)),
            sha256_hex(body.as_bytes()),
        )
        .expect("binary hash");
        fs::write(bin.join(format!("{}.sha256", tool.name)), &tool.sha256).expect("pin");
    }
}

#[test]
fn the_confirmation_tools_are_the_two_pinned_releases() {
    let tools = confirmation_tools();
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        [BUCK2, BAZEL]
    );
    for t in &tools {
        assert!(t.url.starts_with("https://github.com/"), "{}", t.url);
        assert!(t.url.contains("/releases/download/"), "{}", t.url);
        assert_eq!(t.sha256.len(), 64, "{}", t.name);
        assert!(
            t.sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{}",
            t.name
        );
    }
    assert!(tools[0].url.ends_with(".zst"), "Buck2 ships compressed");
    assert!(
        !tools[1].url.ends_with(".zst") && !tools[1].url.ends_with(".tar.gz"),
        "Bazel ships as the program itself"
    );
}

#[test]
fn the_plain_fetch_does_not_download_the_confirmation_tools() {
    let root = scratch("confirm-fetch-all");
    let fake = Fake::default();
    let _ = ensure_all(&root, &fake);
    let log = fake.log.borrow().join("\n");
    for tool in confirmation_tools() {
        assert!(!log.contains(&tool.url), "{} was fetched\n{log}", tool.name);
    }
    let found = ensure_only(
        &root,
        &Fake {
            fail_on: vec!["curl"],
            ..Fake::default()
        },
        &[BUCK2.to_owned(), BAZEL.to_owned()],
    );
    assert_eq!(
        found.iter().map(|f| f.subject.as_str()).collect::<Vec<_>>(),
        [BUCK2, BAZEL]
    );
}

#[test]
fn a_zstd_download_is_decompressed_and_installed_executable() {
    let dir = scratch("confirm-zstd");
    let plain = dir.join("tool-plain");
    fs::write(&plain, "#!/bin/sh\necho zst\n").expect("plain");
    let archive = dir.join("tool-x86_64.zst");
    let compress = Cmd::new(
        "zstd",
        &[
            "--quiet",
            "-o",
            &archive.to_string_lossy(),
            &plain.to_string_lossy(),
        ],
        &dir,
    );
    System.run(&compress).expect("compress");
    let tool = Tool {
        name: "tool".to_owned(),
        url: format!("file://{}", archive.display()),
        sha256: sha256_hex(&fs::read(&archive).expect("archive")),
        member: String::new(),
    };
    let path = ensure(&tool, &bin(&dir), &System).expect("install");
    assert_eq!(
        fs::read_to_string(&path).expect("tool"),
        "#!/bin/sh\necho zst\n"
    );
    let mode = fs::metadata(&path).expect("mode").permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "executable: {mode:o}");
}

#[test]
fn a_download_that_is_the_program_is_installed_executable() {
    let dir = scratch("confirm-raw");
    let source = dir.join("tool-1.0-linux-x86_64");
    fs::write(&source, "#!/bin/sh\necho raw\n").expect("source");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o644)).expect("not executable");
    let tool = Tool {
        name: "tool".to_owned(),
        url: format!("file://{}", source.display()),
        sha256: sha256_hex(&fs::read(&source).expect("source")),
        member: String::new(),
    };
    let path = ensure(&tool, &bin(&dir), &System).expect("install");
    assert_eq!(
        fs::read_to_string(&path).expect("tool"),
        "#!/bin/sh\necho raw\n"
    );
    let mode = fs::metadata(&path).expect("mode").permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "executable: {mode:o}");
}

#[test]
fn a_zstd_download_that_will_not_decompress_names_the_step() {
    let dir = scratch("confirm-zstd-bad");
    let archive = dir.join("tool.zst");
    fs::write(&archive, "this is not zstd data").expect("archive");
    let tool = Tool {
        name: "tool".to_owned(),
        url: format!("file://{}", archive.display()),
        sha256: sha256_hex(&fs::read(&archive).expect("archive")),
        member: String::new(),
    };
    let err = ensure(&tool, &bin(&dir), &System).expect_err("not zstd");
    assert!(err.starts_with("zstd "), "{err}");
}

#[test]
fn the_verb_installs_both_tools_and_runs_the_ignored_tests_with_them_on_the_path() {
    let root = scratch("confirm-verb");
    cache_confirmation_tools(&root);
    let seen = std::rc::Rc::new(std::cell::RefCell::new(None));
    let fake = Fake {
        answers: vec![("cargo test", "OBSERVED: a line of the test\n".to_owned())],
        on_run: Some(Box::new({
            let seen = seen.clone();
            move |cmd| *seen.borrow_mut() = Some(cmd.clone())
        })),
        ..Fake::default()
    };
    let (code, text) = cli::run(&strings(&["build-confirm"]), &none, &fake, &root);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("OBSERVED: a line of the test"), "{text}");
    assert_eq!(*fake.log.borrow(), [TEST_LINE]);
    let cmd = seen.borrow().clone().expect("the test ran");
    assert_eq!(cmd.path_prepend, Some(bin_dir(&root)));
}

#[test]
fn the_verb_stops_when_a_tool_cannot_be_installed() {
    let root = scratch("confirm-verb-nofetch");
    let (code, text) = cli::run(&strings(&["build-confirm"]), &none, &Fake::default(), &root);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("[fetch] buck2:"), "{text}");
    assert!(text.contains("[fetch] bazel:"), "{text}");
}

#[test]
fn a_failing_confirmation_run_fails_the_verb_and_shows_its_output() {
    let root = scratch("confirm-verb-red");
    cache_confirmation_tools(&root);
    let fake = Fake {
        fail_on: vec!["cargo test"],
        ..Fake::default()
    };
    let (code, text) = cli::run(&strings(&["build-confirm"]), &none, &fake, &root);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("[build-confirm]"), "{text}");
    assert!(text.contains("cargo test"), "{text}");
}

#[test]
fn the_usage_lists_the_verb() {
    let (code, text) = cli::run(&strings(&["nope"]), &none, &Fake::default(), Path::new("."));
    assert_eq!(code, 2);
    assert!(text.contains("build-confirm"), "{text}");
}

#[test]
fn the_workflow_is_started_by_hand_with_least_privilege_and_pinned_actions() {
    let text = read(".github/workflows/build-confirm.yml");
    assert!(text.contains("\n  workflow_dispatch:"), "started by hand");
    assert!(
        text.lines().any(|l| l == "permissions: {}"),
        "no permission at the top"
    );
    assert!(
        text.contains("    permissions:\n      contents: read\n"),
        "the job reads the contents and nothing else"
    );
    assert!(!text.contains("secrets."), "no secrets");
    let runs: Vec<&str> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- run: "))
        .collect();
    assert_eq!(runs, ["cargo xtask build-confirm"]);
    let uses: Vec<&str> = text
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- uses: "))
        .collect();
    assert_eq!(uses.len(), 2, "checkout and upload: {uses:?}");
    for u in uses {
        let (_, pin) = u.split_once('@').expect("a version");
        let sha = &pin[..40];
        assert!(
            sha.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "{u} is not pinned by commit"
        );
        assert!(pin[40..].starts_with(" # v"), "{u} names its release");
    }
    // A trigger other than the hand-started one is held to this file's own path.
    if text.contains("\n  pull_request:") {
        assert!(
            text.contains("      - .github/workflows/build-confirm.yml\n"),
            "a pull-request trigger is limited to this workflow's own path"
        );
    }
}

#[test]
fn the_workflow_uploads_what_the_confirmation_test_writes() {
    let text = read(".github/workflows/build-confirm.yml");
    assert!(
        text.contains("if: always()"),
        "transcripts of a red run too"
    );
    assert!(
        text.contains("path: target/tmp/build-confirm/transcripts"),
        "the directory the confirmation test writes"
    );
    let test = read("crates/ucdk-release-build/tests/confirm.rs");
    assert!(test.contains("\"build-confirm\""));
    assert!(test.contains("\"transcripts\""));
}
