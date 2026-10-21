// SPDX-License-Identifier: Apache-2.0
use std::fs;

fn read(path: &str) -> String {
    fs::read_to_string(xtask::workspace_root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn required_public_files_exist() {
    for f in [
        "LICENSE",
        "NOTICE",
        "README.md",
        "SECURITY.md",
        "CONTRIBUTING.md",
        ".github/CODEOWNERS",
        ".github/pull_request_template.md",
        ".github/workflows/ci.yml",
        "deny.toml",
        "rust-toolchain.toml",
    ] {
        assert!(xtask::workspace_root().join(f).is_file(), "{f} must exist");
    }
}

#[test]
fn codeowners_names_only_the_team() {
    let text = read(".github/CODEOWNERS");
    let mut patterns = Vec::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let mut parts = line.split_whitespace();
        patterns.push(parts.next().expect("pattern").to_owned());
        let owners: Vec<_> = parts.collect();
        assert_eq!(owners, ["@komira-ai/ucdk-maintainers"], "line: {line}");
    }
    for p in ["*", "/.github/", "/proto/", "/crates/ucdk-sign/"] {
        assert!(patterns.iter().any(|x| x == p), "CODEOWNERS lacks {p}");
    }
}

#[test]
fn security_md_has_no_email_address() {
    let text = read("SECURITY.md");
    assert!(
        !text.contains('@'),
        "SECURITY.md must not carry an email address"
    );
    assert!(text.contains("private vulnerability reporting"));
}
