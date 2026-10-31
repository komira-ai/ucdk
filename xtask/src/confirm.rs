// SPDX-License-Identifier: Apache-2.0
//! `cargo xtask build-confirm`: the confirmation of the Buck2 and Bazel adapters against the real
//! tools. The verb installs the two pinned releases and runs the ignored tests of
//! `ucdk-release-build/tests/confirm.rs` with the tools on `PATH`; the tests write the raw
//! transcript of every call of a tool under `target/tmp/build-confirm/transcripts`, which the
//! workflow uploads.

use std::path::Path;

use crate::cmd::{Cmd, Runner};
use crate::{Finding, fetch};

/// The test run: the tests of one file, the ignored ones with the rest, in turn so that the
/// printed observations come in order.
fn tests(root: &Path) -> Cmd {
    Cmd::new(
        "cargo",
        &[
            "test",
            "--locked",
            "-p",
            "ucdk-release-build",
            "--test",
            "confirm",
            "--",
            "--include-ignored",
            "--nocapture",
            "--test-threads=1",
        ],
        root,
    )
    .with_path(&fetch::bin_dir(root))
}

/// The verb: the findings, and what the tests printed.
pub fn run(root: &Path, runner: &dyn Runner) -> (Vec<Finding>, String) {
    let names = [fetch::BUCK2.to_owned(), fetch::BAZEL.to_owned()];
    let findings = fetch::ensure_only(root, runner, &names);
    if !findings.is_empty() {
        return (findings, String::new());
    }
    let cmd = tests(root);
    match runner.run(&cmd) {
        Ok(printed) => (Vec::new(), printed),
        Err(e) => (
            vec![Finding::new("build-confirm", cmd.to_string(), e)],
            String::new(),
        ),
    }
}
