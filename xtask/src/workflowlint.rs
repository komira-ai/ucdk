// SPDX-License-Identifier: Apache-2.0
//! Lint for the repository's own GitHub workflows. The rules are the CI contract: only pull
//! requests and pushes to `main` trigger a run, actions are GitHub-owned and pinned by a full
//! commit SHA, the token is read-only, and no untrusted expression is spliced into a script.

use std::fs;
use std::path::Path;

use serde_yaml_ng::Value;

use crate::{Finding, to_result};

fn key<'a>(map: &'a Value, name: &str) -> Option<&'a Value> {
    map.as_mapping()?.get(name)
}

fn is_full_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The `on:` value. YAML 1.1 reads a bare `on` key as `true`, so both are accepted.
fn on_value(doc: &Value) -> Option<&Value> {
    let map = doc.as_mapping()?;
    map.get("on").or_else(|| map.get(Value::Bool(true)))
}

/// Trigger names with their configuration, if any.
fn triggers(on: &Value) -> Vec<(String, Option<&Value>)> {
    match on {
        Value::String(s) => vec![(s.clone(), None)],
        Value::Sequence(seq) => seq
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| (s.to_owned(), None))
            .collect(),
        Value::Mapping(map) => map
            .iter()
            .filter_map(|(k, v)| k.as_str().map(|k| (k.to_owned(), Some(v))))
            .collect(),
        _ => Vec::new(),
    }
}

fn lint_triggers(doc: &Value, out: &mut Vec<Finding>) {
    let Some(on) = on_value(doc) else {
        out.push(Finding::new("trigger", "on", "workflow has no `on:`"));
        return;
    };
    for (name, config) in triggers(on) {
        match name.as_str() {
            "pull_request_target" => out.push(Finding::new(
                "pull-request-target",
                "on",
                "pull_request_target runs with a write token on untrusted code",
            )),
            "pull_request" => {}
            "push" => {
                let branches = config
                    .and_then(|c| key(c, "branches"))
                    .and_then(Value::as_sequence);
                let only_main =
                    branches.is_some_and(|b| b.len() == 1 && b[0].as_str() == Some("main"));
                let extra = config
                    .and_then(Value::as_mapping)
                    .is_some_and(|m| m.keys().any(|k| k.as_str() != Some("branches")));
                if !only_main || extra {
                    out.push(Finding::new(
                        "trigger",
                        "on.push",
                        "push must be `branches: [main]` and nothing else",
                    ));
                }
            }
            other => out.push(Finding::new(
                "trigger",
                format!("on.{other}"),
                "only pull_request and push to main may trigger a run",
            )),
        }
    }
}

fn lint_uses(uses: &str, subject: &str, out: &mut Vec<Finding>) {
    if uses.starts_with("./") || uses.starts_with("docker://") {
        out.push(Finding::new(
            "non-github-owned",
            subject,
            format!("`{uses}` is not a GitHub-owned action"),
        ));
        return;
    }
    let Some((path, reference)) = uses.split_once('@') else {
        out.push(Finding::new(
            "unpinned-action",
            subject,
            format!("`{uses}` has no @ref"),
        ));
        return;
    };
    if path.split('/').next() != Some("actions") {
        out.push(Finding::new(
            "non-github-owned",
            subject,
            format!("`{path}` is not an actions/* action"),
        ));
    }
    if is_full_sha(reference) {
        return;
    }
    let looks_hex = reference.len() >= 7 && reference.bytes().all(|b| b.is_ascii_hexdigit());
    let looks_tag = reference.starts_with('v')
        && reference[1..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        || reference.chars().next().is_some_and(|c| c.is_ascii_digit());
    let (rule, what) = if looks_hex {
        ("short-sha", "an abbreviated commit SHA")
    } else if looks_tag {
        ("tag-ref", "a tag")
    } else {
        ("branch-ref", "a branch")
    };
    out.push(Finding::new(
        rule,
        subject,
        format!("`{uses}` is pinned by {what}; pin the full 40-hex commit SHA"),
    ));
}

fn lint_permissions(perms: Option<&Value>, subject: &str, required: bool, out: &mut Vec<Finding>) {
    let Some(perms) = perms else {
        if required {
            out.push(Finding::new(
                "permissions-missing",
                subject,
                "set `permissions: {contents: read}`",
            ));
        }
        return;
    };
    let ok = match perms {
        Value::Mapping(m) => m
            .iter()
            .all(|(k, v)| k.as_str() == Some("contents") && v.as_str() == Some("read")),
        _ => false,
    };
    if !ok {
        out.push(Finding::new(
            "write-permission",
            subject,
            "permissions may be only `contents: read`",
        ));
    }
}

fn lint_continue_on_error(node: &Value, subject: &str, out: &mut Vec<Finding>) {
    if key(node, "continue-on-error").is_some() {
        out.push(Finding::new(
            "continue-on-error",
            subject,
            "a failing step or job must fail the run",
        ));
    }
}

/// Lints one workflow file's text.
pub fn lint_workflow(yaml: &str) -> Vec<Finding> {
    let doc: Value = match serde_yaml_ng::from_str(yaml) {
        Ok(v) => v,
        Err(e) => return vec![Finding::new("yaml", "workflow", e.to_string())],
    };
    let mut out = Vec::new();
    lint_triggers(&doc, &mut out);
    lint_permissions(key(&doc, "permissions"), "permissions", true, &mut out);
    let Some(jobs) = key(&doc, "jobs").and_then(Value::as_mapping) else {
        out.push(Finding::new("jobs", "jobs", "workflow has no jobs"));
        return out;
    };
    for (job_id, job) in jobs {
        let jid = job_id.as_str().unwrap_or("?");
        let subject = format!("jobs.{jid}");
        lint_permissions(
            key(job, "permissions"),
            &format!("{subject}.permissions"),
            false,
            &mut out,
        );
        lint_continue_on_error(job, &subject, &mut out);
        if let Some(uses) = key(job, "uses").and_then(Value::as_str) {
            lint_uses(uses, &subject, &mut out);
        }
        let steps = key(job, "steps")
            .and_then(Value::as_sequence)
            .map_or(&[][..], Vec::as_slice);
        for (i, step) in steps.iter().enumerate() {
            let subject = format!("{subject}.steps[{i}]");
            lint_continue_on_error(step, &subject, &mut out);
            if let Some(uses) = key(step, "uses").and_then(Value::as_str) {
                lint_uses(uses, &subject, &mut out);
            }
            if let Some(run) = key(step, "run").and_then(Value::as_str)
                && run.contains("${{")
            {
                out.push(Finding::new(
                    "expression-in-run",
                    subject,
                    "pass the value through `env:` and read it as a shell variable",
                ));
            }
        }
    }
    out
}

/// The `workflowlint` verb: every file under `.github/workflows`.
pub fn run(root: &Path) -> Result<(), Vec<Finding>> {
    let dir = root.join(".github/workflows");
    let mut findings = Vec::new();
    let entries = fs::read_dir(&dir)
        .map_err(|e| vec![Finding::new("io", ".github/workflows", e.to_string())])?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !matches!(
            path.extension().and_then(|x| x.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        match fs::read_to_string(&path) {
            Ok(text) => findings.extend(lint_workflow(&text).into_iter().map(|mut f| {
                f.subject = format!("{name}: {}", f.subject);
                f
            })),
            Err(e) => findings.push(Finding::new("io", name, e.to_string())),
        }
    }
    to_result(findings)
}
