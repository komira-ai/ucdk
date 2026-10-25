// SPDX-License-Identifier: Apache-2.0
//! Tool installation: pinned, verified before unpacking, cached by pin.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::{Fake, scratch, write};
use xtask::cmd::{Cmd, Runner, System};
use xtask::fetch::{Tool, bin_dir, ensure, ensure_all, pinned, sha256_hex};

/// A tool whose archive is built here, served from disk through a `file://` URL.
fn local_tool(dir: &Path, member_in_archive: &str) -> Tool {
    let payload = dir.join("payload");
    write(&payload, "inner/my-tool", "#!/bin/sh\necho hi\n");
    let archive = dir.join("my-tool.tar.gz");
    let tar = Cmd::new(
        "tar",
        &[
            "-czf",
            &archive.to_string_lossy(),
            "-C",
            &payload.to_string_lossy(),
            "inner",
        ],
        dir,
    );
    System.run(&tar).expect("build the archive");
    Tool {
        name: "my-tool".to_owned(),
        url: format!("file://{}", archive.display()),
        sha256: sha256_hex(&fs::read(&archive).expect("read archive")),
        member: member_in_archive.to_owned(),
    }
}

fn bin(dir: &Path) -> PathBuf {
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).expect("bin dir");
    bin
}

#[test]
fn a_pinned_download_is_verified_unpacked_and_installed() {
    let dir = scratch("fetch-ok");
    let tool = local_tool(&dir, "inner/my-tool");
    let path = ensure(&tool, &bin(&dir), &System).expect("install");
    assert_eq!(
        fs::read_to_string(&path).expect("tool"),
        "#!/bin/sh\necho hi\n"
    );
    assert!(
        !dir.join("bin/my-tool.work").exists(),
        "the work directory is removed"
    );
}

#[test]
fn an_installed_tool_is_not_downloaded_again_but_a_new_pin_is() {
    let dir = scratch("fetch-cached");
    let tool = local_tool(&dir, "inner/my-tool");
    let bin = bin(&dir);
    ensure(&tool, &bin, &System).expect("install");
    fs::remove_file(dir.join("my-tool.tar.gz")).expect("remove the source");
    let fake = Fake::default();
    ensure(&tool, &bin, &fake).expect("cached");
    assert!(fake.log.borrow().is_empty(), "nothing ran");
    let moved = Tool {
        sha256: "0".repeat(64),
        ..tool
    };
    assert!(
        ensure(&moved, &bin, &System).is_err(),
        "a changed pin fetches again, and fails here"
    );
}

#[test]
fn a_download_with_the_wrong_hash_is_refused_before_unpacking() {
    let dir = scratch("fetch-mismatch");
    let tool = Tool {
        sha256: "0".repeat(64),
        ..local_tool(&dir, "inner/my-tool")
    };
    let err = ensure(&tool, &bin(&dir), &System).expect_err("hash mismatch");
    assert!(err.contains("the pin is"), "{err}");
    assert!(!dir.join("bin/my-tool").exists());
    assert!(
        !dir.join("bin/my-tool.work/unpacked/inner").exists(),
        "nothing was unpacked"
    );
}

#[test]
fn fetch_errors_name_the_step_that_failed() {
    let dir = scratch("fetch-errors");
    let bin = bin(&dir);
    let tool = local_tool(&dir, "inner/missing");
    let err = ensure(&tool, &bin, &System).expect_err("member missing");
    assert!(err.contains("No such file"), "{err}");

    let gone = Tool {
        url: format!("file://{}/nope.tar.gz", dir.display()),
        ..tool.clone()
    };
    assert!(ensure(&gone, &bin, &System).is_err(), "download fails");

    let broken = Fake {
        on_run: Some(Box::new({
            let archive = dir.join("my-tool.tar.gz");
            let target = bin.join("my-tool.work/archive.tar.gz");
            move |cmd| {
                if cmd.program == "curl" {
                    fs::copy(&archive, &target).expect("stage the archive");
                }
            }
        })),
        fail_on: vec!["-xzf"],
        ..Fake::default()
    };
    let err = ensure(&tool, &bin, &broken).expect_err("tar fails");
    assert!(err.contains("-xzf"), "{err}");

    let blocked = dir.join("blocked");
    fs::write(&blocked, "a file, not a directory").expect("write");
    assert!(
        ensure(&tool, &blocked, &System).is_err(),
        "cannot create the work directory"
    );
}

#[test]
fn ensure_all_installs_each_pinned_tool_and_reports_each_failure() {
    let root = scratch("fetch-all");
    let fake = Fake {
        fail_on: vec!["curl"],
        ..Fake::default()
    };
    let found = ensure_all(&root, &fake);
    assert_eq!(
        found.iter().map(|f| f.subject.as_str()).collect::<Vec<_>>(),
        ["cargo-deny", "cargo-llvm-cov", "node"]
    );

    let blocked = scratch("fetch-all-blocked");
    write(
        &blocked,
        "target/xtask-tools",
        "a file where the directory should be",
    );
    let found = ensure_all(&blocked, &Fake::default());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].subject, "tools");
}

#[test]
fn the_pins_are_https_release_urls_with_full_hashes() {
    let tools = pinned();
    assert_eq!(tools.len(), 3);
    for t in &tools {
        assert!(
            t.url.starts_with("https://github.com/")
                || t.url.starts_with("https://nodejs.org/dist/"),
            "{}",
            t.url
        );
        assert_eq!(t.sha256.len(), 64, "{}", t.name);
        assert!(
            t.sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert!(t.member.ends_with(&t.name), "{}", t.member);
    }
    assert!(bin_dir(Path::new("/r")).ends_with("target/xtask-tools/bin"));
}

#[test]
fn sha256_matches_the_standard_vectors() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn a_binary_changed_after_install_is_fetched_again() {
    let dir = scratch("fetch-rehash");
    let tool = local_tool(&dir, "inner/my-tool");
    let bin = bin(&dir);
    ensure(&tool, &bin, &System).expect("install");
    fs::write(bin.join("my-tool"), "something else").expect("replace the binary");
    let fake = Fake::default();
    let _ = ensure(&tool, &bin, &fake);
    assert!(
        fake.log.borrow().iter().any(|c| c.starts_with("curl")),
        "a replaced binary is downloaded again: {:?}",
        fake.log.borrow()
    );
    // Installing for real restores the pinned binary.
    ensure(&tool, &bin, &System).expect("reinstall");
    assert_eq!(
        fs::read_to_string(bin.join("my-tool")).expect("tool"),
        "#!/bin/sh\necho hi\n"
    );
    // A missing hash record counts as a changed binary too.
    fs::remove_file(bin.join("my-tool.bin-sha256")).expect("drop the record");
    let fake = Fake::default();
    let _ = ensure(&tool, &bin, &fake);
    assert!(!fake.log.borrow().is_empty());
}

#[test]
fn ensure_only_installs_the_tools_it_names() {
    let root = scratch("fetch-only");
    let fake = Fake {
        fail_on: vec!["curl"],
        ..Fake::default()
    };
    let found = xtask::fetch::ensure_only(&root, &fake, &["cargo-llvm-cov".to_owned()]);
    assert_eq!(
        found.iter().map(|f| f.subject.as_str()).collect::<Vec<_>>(),
        ["cargo-llvm-cov"]
    );
    assert!(fake.log.borrow().iter().all(|c| !c.contains("cargo-deny")));
}
