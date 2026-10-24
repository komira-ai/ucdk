// SPDX-License-Identifier: Apache-2.0
//! `deny.toml` refuses what it names. The test runs the pinned cargo-deny, with the committed
//! `deny.toml`, on two small fixture workspaces. The clean one must pass first, so a missing tool
//! or a broken configuration cannot make the refusal look like a pass.

mod common;

use std::path::{Path, PathBuf};

use common::scratch;
use xtask::cmd::{Cmd, Runner, System};
use xtask::fetch::{bin_dir, ensure_all};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create dir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let dest = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).expect("copy");
        }
    }
}

fn deny(fixture: &str, case: &str) -> Result<String, String> {
    let root = xtask::workspace_root();
    let tools = ensure_all(&root, &System);
    assert!(tools.is_empty(), "{tools:?}");
    let dir = scratch(case);
    copy_dir(&fixtures().join(fixture), &dir);
    let config = root.join("deny.toml");
    let cmd = Cmd::new(
        "cargo",
        &[
            "deny",
            "--log-level",
            "warn",
            "--manifest-path",
            &dir.join("Cargo.toml").to_string_lossy(),
            "--all-features",
            "--config",
            &config.to_string_lossy(),
            "check",
            "licenses",
            "bans",
            "sources",
        ],
        &dir,
    )
    .with_path(&bin_dir(&root));
    System.run(&cmd)
}

#[test]
fn deny_rejects_unlisted_licence() {
    deny("clean", "deny-clean").unwrap_or_else(|e| panic!("the clean fixture must pass:\n{e}"));
    let err = deny("copyleft", "deny-copyleft").expect_err("a GPL dependency must be refused");
    assert!(err.contains("GPL-3.0-only"), "{err}");
}
