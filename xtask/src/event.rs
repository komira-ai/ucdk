// SPDX-License-Identifier: Apache-2.0
//! The pull-request event file the platform provides to a workflow run.

use serde_json::Value;

use crate::Finding;

/// What the checks need from the event: whether this run is a pull request, and its base commit,
/// title and body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Event {
    pub is_pull_request: bool,
    pub base_sha: String,
    pub title: String,
    pub body: String,
}

fn text<'a>(v: &'a Value, pointer: &str) -> &'a str {
    v.pointer(pointer).and_then(Value::as_str).unwrap_or("")
}

pub fn parse_event(json: &str) -> Result<Event, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("event file: {e}"))?;
    Ok(Event {
        is_pull_request: v.get("pull_request").is_some(),
        base_sha: text(&v, "/pull_request/base/sha").to_owned(),
        title: text(&v, "/pull_request/title").to_owned(),
        body: text(&v, "/pull_request/body").to_owned(),
    })
}

/// Whether `base` is a commit id or a ref name: it does not start with `-` (git would read it as
/// an option) and has only letters, digits and `. _ / - @ ~ ^`.
fn is_commit_or_ref(base: &str) -> bool {
    !base.starts_with('-')
        && base
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-@~^".contains(c))
}

/// The commit range base for the commit checks. A `--base` flag wins over the event. A pull
/// request must always have one, so a pull request with an empty base is a finding, and so is a
/// base that is not a commit id or a ref name; any other run
/// (a push, a local run) has no base and the commit checks are skipped.
pub fn commit_base(event: &Event, flag: Option<&str>) -> Result<Option<String>, Finding> {
    let base = match flag {
        Some(b) if !b.is_empty() => b.to_owned(),
        _ => event.base_sha.clone(),
    };
    match (base.is_empty(), event.is_pull_request) {
        (false, _) if !is_commit_or_ref(&base) => Err(Finding::new(
            "dco-bad-base",
            "base",
            format!("{base:?} is neither a commit id nor a ref name"),
        )),
        (false, _) => Ok(Some(base)),
        (true, true) => Err(Finding::new(
            "dco-no-base",
            "pull request",
            "no base commit was found, so no commit would be checked",
        )),
        (true, false) => Ok(None),
    }
}
