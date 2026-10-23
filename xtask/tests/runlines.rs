// SPDX-License-Identifier: Apache-2.0
use xtask::runlines::check_run_lines;

fn workflow(text: &str) -> Vec<(String, String)> {
    vec![(".github/workflows/ci.yml".to_owned(), text.to_owned())]
}

fn found(text: &str) -> Vec<String> {
    check_run_lines(&workflow(text))
        .into_iter()
        .map(|f| f.subject)
        .collect()
}

#[test]
fn run_line_with_expression_is_rejected() {
    let expression = "steps:
  - run: cargo xtask check --base ${{ github.sha }}
";
    assert_eq!(found(expression), [".github/workflows/ci.yml:2"]);
    let interpolated = "steps:
  - name: x
    run: echo ${{ github.event.pull_request.title }}
";
    assert_eq!(found(interpolated), [".github/workflows/ci.yml:3"]);
}

#[test]
fn run_lines_must_be_exactly_one_xtask_verb() {
    for bad in [
        "run: cargo test",
        "run: cargo xtask",
        "run: cargo xtask Check",
        "run: cargo xtask check && cargo xtask coverage",
        "run: |",
        "run:",
        "- run: rustup toolchain install",
    ] {
        assert_eq!(found(bad).len(), 1, "{bad}");
    }
    for good in [
        "run: cargo xtask check",
        "  - run: cargo xtask coverage",
        "    run:   cargo xtask fetch-all  ",
        "name: runs on push\nuses: actions/checkout@abc",
    ] {
        assert!(found(good).is_empty(), "{good}");
    }
}

#[test]
fn only_workflow_files_are_read() {
    let files = vec![
        ("README.md".to_owned(), "run: cargo test".to_owned()),
        (
            ".github/workflows/notes.txt".to_owned(),
            "run: cargo test".to_owned(),
        ),
        (
            ".github/workflows/b.yaml".to_owned(),
            "run: cargo test".to_owned(),
        ),
    ];
    let subjects: Vec<_> = check_run_lines(&files)
        .into_iter()
        .map(|f| f.subject)
        .collect();
    assert_eq!(subjects, [".github/workflows/b.yaml:1"]);
}
