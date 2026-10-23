// SPDX-License-Identifier: Apache-2.0
mod common;

use common::scratch;
use xtask::cmd::{Cmd, Runner, System};

#[test]
fn a_command_prints_as_its_command_line() {
    let cmd = Cmd::new("git", &["log", "--oneline"], &scratch("cmd-display"));
    assert_eq!(cmd.to_string(), "git log --oneline");
}

#[test]
fn system_returns_standard_output_or_the_error_text() {
    let dir = scratch("cmd-system");
    let ok = System
        .run(&Cmd::new("git", &["--version"], &dir))
        .expect("git runs");
    assert!(ok.starts_with("git version"), "{ok}");
    let err = System
        .run(&Cmd::new("git", &["no-such-subcommand"], &dir))
        .expect_err("fails");
    assert!(err.starts_with("git no-such-subcommand: "), "{err}");
    assert!(err.contains("not a git command"), "{err}");
    let err = System
        .run(&Cmd::new("no-such-program-xyz", &[], &dir))
        .expect_err("cannot spawn");
    assert!(err.starts_with("no-such-program-xyz: "), "{err}");
}

#[test]
fn a_prepended_directory_comes_first_on_the_path() {
    let dir = scratch("cmd-path");
    let tools = dir.join("bin");
    std::fs::create_dir_all(&tools).expect("bin");
    // The program is looked up on the child's PATH: put a real executable there under a new name.
    std::fs::copy(
        std::env::current_exe().expect("this test binary"),
        tools.join("my-extra"),
    )
    .expect("copy");
    let found = Cmd::new("my-extra", &["--list"], &dir).with_path(&tools);
    assert_eq!(found.path_prepend.as_deref(), Some(tools.as_path()));
    assert!(
        System.run(&found).is_ok(),
        "found through the prepended directory"
    );
    assert!(
        System
            .run(&Cmd::new("my-extra", &["--list"], &dir))
            .is_err(),
        "not on the ordinary PATH"
    );
}
