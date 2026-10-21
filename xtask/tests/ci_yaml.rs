// SPDX-License-Identifier: Apache-2.0
//! The committed workflow honours the contract other tooling relies on: the workflow is named
//! `ci` and has exactly one job named `check`, the name branch protection will require.

use serde_json::Value;

fn ci() -> Value {
    let path = xtask::workspace_root().join(".github/workflows/ci.yml");
    serde_saphyr::from_str(&std::fs::read_to_string(path).expect("ci.yml")).expect("yaml")
}

fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    v.as_object()
        .and_then(|m| m.get(k))
        .unwrap_or_else(|| panic!("missing key {k}"))
}

#[test]
fn workflow_named_ci_job_named_check() {
    let doc = ci();
    assert_eq!(get(&doc, "name").as_str(), Some("ci"));
    let jobs = get(&doc, "jobs").as_object().expect("jobs map");
    assert_eq!(jobs.len(), 1, "exactly one job");
    let (id, job) = jobs.iter().next().expect("one job");
    assert_eq!(id, "check");
    assert_eq!(get(job, "name").as_str(), Some("check"));
}

#[test]
fn triggers_include_edited() {
    let doc = ci();
    let on = doc.as_object().and_then(|m| m.get("on")).expect("on");
    let types = get(get(on, "pull_request"), "types")
        .as_array()
        .expect("types");
    let types: Vec<_> = types.iter().filter_map(Value::as_str).collect();
    assert_eq!(types, ["opened", "synchronize", "reopened", "edited"]);
}

#[test]
fn job_runs_cargo_xtask_check() {
    let doc = ci();
    let job = get(get(&doc, "jobs"), "check");
    let steps = get(job, "steps").as_array().expect("steps");
    let runs: Vec<_> = steps
        .iter()
        .filter_map(|s| s.as_object()?.get("run")?.as_str())
        .collect();
    let checks = runs
        .iter()
        .filter(|r| r.starts_with("cargo xtask check"))
        .count();
    assert_eq!(
        checks, 1,
        "exactly one step runs cargo xtask check: {runs:?}"
    );
}
