// SPDX-License-Identifier: Apache-2.0
use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ucdk"))
        .args(args)
        .output()
        .expect("run ucdk")
}

#[test]
fn version_prints_name_and_version() {
    let out = run(&["--version"]);
    assert!(out.status.success());
    let want = format!("ucdk {}\n", env!("CARGO_PKG_VERSION"));
    assert_eq!(String::from_utf8_lossy(&out.stdout), want);
}

#[test]
fn any_other_argument_is_a_usage_error() {
    let out = run(&["plan"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}
