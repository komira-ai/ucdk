// SPDX-License-Identifier: Apache-2.0
//! The bodies of the reads that list: the versions of a package, a page of a search, and the
//! package file of the sparse index. As in [`crate::answers`], a body of any other shape is `None`
//! and the caller makes a schema violation of it.

use std::collections::BTreeSet;

use serde_json::Value;

use crate::answers::{VersionRecord, VersionState, parse_record};

/// The versions `GET /packages/{name}` lists, newest first, yanked and tombstoned ones included:
/// each with the state the package record gives it.
pub fn parse_package(body: &[u8]) -> Option<Vec<(String, VersionState)>> {
    let value: Value = serde_json::from_slice(body).ok()?;
    value
        .get("versions")
        .and_then(Value::as_array)
        .and_then(|versions| {
            versions
                .iter()
                .map(|v| {
                    let version = v.get("version").and_then(Value::as_str);
                    let state = v
                        .get("state")
                        .and_then(Value::as_str)
                        .and_then(VersionState::parse);
                    version.zip(state).map(|(v, s)| (v.to_owned(), s))
                })
                .collect()
        })
}

/// One page of `GET /packages`: each package with its latest published version, if it has one,
/// and the cursor of the next page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub packages: Vec<(String, Option<String>)>,
    pub next_cursor: Option<String>,
}

pub fn parse_page(body: &[u8]) -> Option<Page> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let packages = value
        .get("packages")
        .and_then(Value::as_array)
        .and_then(|packages| {
            packages
                .iter()
                .map(|p| {
                    let name = p.get("name").and_then(Value::as_str)?;
                    let latest = match p.get("latest") {
                        Some(Value::Null) => Some(None),
                        Some(Value::String(v)) => Some(Some(v.clone())),
                        _ => None,
                    };
                    latest.map(|latest| (name.to_owned(), latest))
                })
                .collect()
        });
    let next = match value.get("next_cursor") {
        Some(Value::Null) => Some(None),
        Some(Value::String(cursor)) => Some(Some(cursor.clone())),
        _ => None,
    };
    match (packages, next) {
        (Some(packages), Some(next_cursor)) => Some(Page {
            packages,
            next_cursor,
        }),
        _ => None,
    }
}

/// The package file of the sparse index: one JSON record per line, each ending in a newline (a
/// file that does not end in one was truncated), every line of the record version this crate
/// reads and naming `name`, and no version twice. Unknown keys are ignored.
pub fn parse_index(body: &[u8], name: &str) -> Option<Vec<VersionRecord>> {
    let text = std::str::from_utf8(body).ok()?.strip_suffix('\n')?;
    let mut seen = BTreeSet::new();
    text.split('\n')
        .map(|line| {
            let value: Value = serde_json::from_str(line).ok()?;
            let known = value.get("v").and_then(Value::as_u64) == Some(1);
            let record = parse_record(line.as_bytes())?;
            (known && record.name == name && seen.insert(record.version.clone())).then_some(record)
        })
        .collect()
}

/// Whether `prefix` is one mopm's search takes: at most 64 characters, each a letter that is lower
/// case, a digit or an underscore, so that a name's prefix is what the registry matches. The empty
/// prefix lists every package.
pub fn valid_prefix(prefix: &str) -> bool {
    prefix.len() <= 64
        && prefix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
