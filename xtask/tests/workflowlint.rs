// SPDX-License-Identifier: Apache-2.0
use xtask::workflowlint::lint_workflow;

const SHA: &str = "3d3c42e5aac5ba805825da76410c181273ba90b1";

/// A workflow that passes every rule; each test replaces one part of it.
fn workflow(on: &str, perms: &str, step: &str) -> String {
    format!(
        "name: ci\non:\n{on}\npermissions:\n{perms}\njobs:\n  check:\n    runs-on: ubuntu-24.04\n    steps:\n{step}\n"
    )
}

const ON: &str = "  pull_request:\n  push:\n    branches: [main]";
const PERMS: &str = "  contents: read";
const STEP: &str = "      - run: cargo xtask check";

fn uses_step(uses: &str) -> String {
    format!("      - uses: {uses}")
}

fn rules(yaml: &str) -> Vec<&'static str> {
    lint_workflow(yaml).into_iter().map(|f| f.rule).collect()
}

fn has(yaml: &str, rule: &str) -> bool {
    rules(yaml).contains(&rule)
}

#[test]
fn baseline_is_green() {
    assert_eq!(lint_workflow(&workflow(ON, PERMS, STEP)), vec![]);
}

#[test]
fn pull_request_target_red() {
    // As a map key under `on:`.
    assert!(has(
        &workflow("  pull_request_target:", PERMS, STEP),
        "pull-request-target"
    ));
    // As a list entry.
    let list = format!(
        "name: ci\non: [pull_request_target]\npermissions:\n{PERMS}\njobs:\n  check:\n    runs-on: x\n    steps:\n{STEP}\n"
    );
    assert!(has(&list, "pull-request-target"));
    // As a scalar.
    let scalar = format!(
        "name: ci\non: pull_request_target\npermissions:\n{PERMS}\njobs:\n  check:\n    runs-on: x\n    steps:\n{STEP}\n"
    );
    assert!(has(&scalar, "pull-request-target"));
}

#[test]
fn trigger_other_than_pr_and_push_main_red() {
    for on in [
        "  pull_request:\n  schedule:\n    - cron: '0 0 * * *'",
        "  pull_request:\n  workflow_dispatch:",
        "  push:\n    branches: [main, dev]",
        "  push:\n    branches: [dev]",
        "  push:\n    tags: ['v*']",
        "  push:",
    ] {
        assert!(has(&workflow(on, PERMS, STEP), "trigger"), "{on}");
    }
}

#[test]
fn tag_ref_action_red() {
    assert!(has(
        &workflow(ON, PERMS, &uses_step("actions/checkout@v4")),
        "tag-ref"
    ));
    assert!(has(
        &workflow(ON, PERMS, &uses_step("actions/checkout@v7.0.1")),
        "tag-ref"
    ));
}

#[test]
fn branch_ref_action_red() {
    assert!(has(
        &workflow(ON, PERMS, &uses_step("actions/checkout@main")),
        "branch-ref"
    ));
    assert!(has(
        &workflow(ON, PERMS, &uses_step("actions/checkout")),
        "unpinned-action"
    ));
}

#[test]
fn short_sha_red() {
    assert!(has(
        &workflow(ON, PERMS, &uses_step("actions/checkout@3d3c42e")),
        "short-sha"
    ));
    assert!(has(
        &workflow(
            ON,
            PERMS,
            &uses_step(&format!("actions/checkout@{}", &SHA[..39]))
        ),
        "short-sha"
    ));
    assert_eq!(
        lint_workflow(&workflow(
            ON,
            PERMS,
            &uses_step(&format!("actions/checkout@{SHA}"))
        )),
        vec![]
    );
}

#[test]
fn non_github_owned_action_red() {
    let step = uses_step(&format!("dtolnay/rust-toolchain@{SHA}"));
    assert!(has(&workflow(ON, PERMS, &step), "non-github-owned"));
    assert!(has(
        &workflow(ON, PERMS, &uses_step("./local-action")),
        "non-github-owned"
    ));
    assert!(has(
        &workflow(ON, PERMS, &uses_step("docker://alpine:3")),
        "non-github-owned"
    ));
    let cache = uses_step(&format!("actions/cache/restore@{SHA}"));
    assert_eq!(lint_workflow(&workflow(ON, PERMS, &cache)), vec![]);
}

#[test]
fn expression_in_run_red() {
    let step = "      - run: echo ${{ github.event.pull_request.title }}";
    assert!(has(&workflow(ON, PERMS, step), "expression-in-run"));
}

#[test]
fn expression_in_env_green() {
    let step = "      - run: echo \"$TITLE\"\n        env:\n          TITLE: ${{ github.event.pull_request.title }}";
    assert_eq!(lint_workflow(&workflow(ON, PERMS, step)), vec![]);
}

#[test]
fn write_permission_red() {
    assert!(has(
        &workflow(ON, "  contents: write", STEP),
        "write-permission"
    ));
    assert!(has(
        &workflow(ON, "  contents: read\n  pull-requests: write", STEP),
        "write-permission"
    ));
    assert!(has(
        &workflow(ON, "  id-token: write", STEP),
        "write-permission"
    ));
    // At job level.
    let job = "name: ci\non: pull_request\npermissions:\n  contents: read\njobs:\n  check:\n    runs-on: x\n    permissions:\n      contents: write\n    steps:\n      - run: x\n";
    assert!(has(job, "write-permission"));
    // Missing at the top level.
    let none =
        "name: ci\non: pull_request\njobs:\n  check:\n    runs-on: x\n    steps:\n      - run: x\n";
    assert!(has(none, "permissions-missing"));
}

#[test]
fn continue_on_error_red() {
    let step = "      - run: cargo xtask check\n        continue-on-error: true";
    assert!(has(&workflow(ON, PERMS, step), "continue-on-error"));
    let job = "name: ci\non: pull_request\npermissions:\n  contents: read\njobs:\n  check:\n    runs-on: x\n    continue-on-error: true\n    steps:\n      - run: x\n";
    assert!(has(job, "continue-on-error"));
}

#[test]
fn own_ci_yml_green() {
    let text = std::fs::read_to_string(xtask::workspace_root().join(".github/workflows/ci.yml"))
        .expect("ci.yml");
    assert_eq!(lint_workflow(&text), vec![]);
    assert_eq!(xtask::workflowlint::run(&xtask::workspace_root()), Ok(()));
}
