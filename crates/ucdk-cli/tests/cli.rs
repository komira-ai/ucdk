// SPDX-License-Identifier: Apache-2.0
use std::process::Command;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn run_lib(list: &[&str]) -> (u8, String, String) {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = ucdk_cli::run(&args(list), &mut out, &mut err);
    (
        code,
        String::from_utf8(out).expect("utf8"),
        String::from_utf8(err).expect("utf8"),
    )
}

#[test]
fn cli_prints_version() {
    let (code, out, err) = run_lib(&["--version"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(out, format!("ucdk {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn cli_rejects_any_other_arguments() {
    for list in [&[][..], &["plan"], &["--version", "extra"]] {
        let (code, out, err) = run_lib(list);
        assert_eq!((code, out.as_str()), (2, ""), "{list:?}");
        assert_eq!(err, "usage: ucdk --version\n");
    }
}

#[test]
fn binary_prints_version_and_exits_zero() {
    let out = Command::new(env!("CARGO_BIN_EXE_ucdk"))
        .arg("--version")
        .output()
        .expect("run ucdk");
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("ucdk {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn binary_exits_two_on_a_usage_error() {
    let out = Command::new(env!("CARGO_BIN_EXE_ucdk"))
        .arg("plan")
        .output()
        .expect("run ucdk");
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}
