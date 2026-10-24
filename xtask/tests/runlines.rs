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

#[test]
fn a_run_key_is_found_however_the_yaml_spells_it() {
    for bad in [
        "      -   run: curl x",
        "      - {run: curl x}",
        "      \"run\": curl x",
        "      'run': x",
        "      run : x",
        "    steps: [{run: x}]",
        "    steps: [{name: a, run: x}]",
        "  - run: cargo xtask check  # then more\n",
        "  - {run: cargo xtask check}",
    ] {
        assert_eq!(found(bad).len(), 1, "{bad}");
    }
    for good in [
        "# run: curl x",
        "    - uses: a/b@c\n      with: {runner: x}",
        "      name: run",
        "      name: the run, then",
    ] {
        assert!(found(good).is_empty(), "{good}");
    }
}

#[test]
fn composite_actions_are_read_too() {
    let files = vec![
        (
            ".github/actions/setup/action.yml".to_owned(),
            "runs:\n  steps:\n    - run: curl x\n".to_owned(),
        ),
        (
            ".github/actions/setup/notes.md".to_owned(),
            "run: curl x".to_owned(),
        ),
    ];
    let subjects: Vec<_> = check_run_lines(&files)
        .into_iter()
        .map(|f| f.subject)
        .collect();
    assert_eq!(subjects, [".github/actions/setup/action.yml:3"]);
}
