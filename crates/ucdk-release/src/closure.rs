// SPDX-License-Identifier: Apache-2.0
//! The `SET_CLOSURE` validation: a pure function of the set document. It has one derived row
//! per member, `<member>: every requirement names a member of the set`. An image member declares
//! nothing and passes. A FILE member declares through two labels, each a comma-separated list
//! (names are trimmed and an empty name between commas is none):
//!
//! - `requires`: the members it needs. A requirement that names no member of the set is a failed
//!   row naming it.
//! - `members`: the members it brings in. Its presence makes the member a list, and the names
//!   must be exactly the plain members of the set, the members that have a manifest and no
//!   `members` label of their own. A plain member the list leaves out, a name that is not a
//!   plain member, and a name listed twice are each a failed row naming them.
//!
//! An entry of either list is a bare name or an exact pin, `name==version`. A pin holds only
//! when the named member's manifest `version` is, byte for byte, the pinned one: a prefix of it,
//! an extension of it and a member with no version do not match. Any other use of a comparison
//! character (`<`, `>`, `=`, `~`, `!`, `*`) is a range, a wildcard or a loose match and is a
//! failed row naming the entry.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{Manifest, Member, ReleaseSet, Row, RowState};

use crate::rows::ExpectedRow;

/// The label that lists a member's requirements.
pub const REQUIRES: &str = "requires";

/// The label that lists the members a member brings in, which must be exactly the plain members
/// of the set.
pub const MEMBERS: &str = "members";

/// What every row of the closure asserts.
pub const ASSERTION: &str = "every requirement names a member of the set";

/// What a row of a member with no manifest is about, since it has no name.
fn subject(at: usize, member: &Member) -> String {
    member
        .manifest
        .as_ref()
        .map_or_else(|| format!("member {at}"), |m| m.name.clone())
}

/// The rows the closure derives from `set`, before anything is evaluated: one per member, in
/// the order of the set.
pub fn closure_rows(set: &ReleaseSet) -> Vec<ExpectedRow> {
    set.members
        .iter()
        .enumerate()
        .map(|(at, member)| ExpectedRow::new(subject(at, member), ASSERTION))
        .collect()
}

/// The characters that begin a version comparison. A name never holds one.
const COMPARISONS: [char; 6] = ['<', '>', '=', '~', '!', '*'];

/// One entry of a `requires` or `members` list.
enum Entry<'a> {
    /// A name with no pin.
    Bare(&'a str),
    /// A name and the one version it must have.
    Pinned { name: &'a str, version: &'a str },
    /// Anything else that uses a comparison character. `name` is what comes before it, which is
    /// empty when there is nothing.
    Loose { entry: &'a str, name: &'a str },
}

fn parse(entry: &str) -> Entry<'_> {
    let Some(at) = entry.find(COMPARISONS) else {
        return Entry::Bare(entry);
    };
    let name = entry[..at].trim();
    let version = entry[at..]
        .strip_prefix("==")
        .map(str::trim)
        .filter(|version| {
            !version.is_empty()
                && !version.contains(|c: char| COMPARISONS.contains(&c) || c.is_whitespace())
        });
    match version {
        Some(version) if !name.is_empty() => Entry::Pinned { name, version },
        _ => Entry::Loose { entry, name },
    }
}

/// The entries of the label `key` of `manifest`: `None` when the manifest has no such label.
fn entries<'a>(manifest: &'a Manifest, key: &str) -> Option<Vec<&'a str>> {
    let Some(Metadata::FileMeta(meta)) = &manifest.metadata else {
        return None;
    };
    meta.labels.get(key).map(|list| {
        list.split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect()
    })
}

/// What a list says, set against the members it may name.
#[derive(Default)]
struct Resolved<'a> {
    /// The names the list holds, whether or not they are members.
    listed: BTreeSet<&'a str>,
    /// Names that are not among the members, once each, in the order of the list.
    unknown: Vec<&'a str>,
    /// Entries that are neither a bare name nor an exact pin.
    loose: Vec<String>,
    /// Pins that a member's version does not equal, as a sentence each.
    mismatched: Vec<String>,
    /// Names that appear more than once, once each.
    repeated: Vec<&'a str>,
}

/// Sets the `entries` of a list, introduced by `verb` in its sentences, against `versions`: the
/// members it may name, with the version of each.
fn resolve<'a>(verb: &str, list: &[&'a str], versions: &BTreeMap<&str, &str>) -> Resolved<'a> {
    let mut resolved = Resolved::default();
    for entry in list {
        let (name, pin) = match parse(entry) {
            Entry::Bare(name) => (name, None),
            Entry::Pinned { name, version } => (name, Some(version)),
            Entry::Loose { entry, name } => {
                resolved.loose.push(format!(
                    "{entry} is not a bare name or an exact name==version pin"
                ));
                (name, None)
            }
        };
        if name.is_empty() {
            continue;
        }
        let first = resolved.listed.insert(name);
        if !first && !resolved.repeated.contains(&name) {
            resolved.repeated.push(name);
        }
        let Some(have) = versions.get(name) else {
            if first {
                resolved.unknown.push(name);
            }
            continue;
        };
        if let Some(want) = pin
            && want != *have
        {
            let at = if have.is_empty() { "no version" } else { *have };
            resolved
                .mismatched
                .push(format!("{verb} {name}=={want}, but {name} is at {at}"));
        }
    }
    resolved
}

/// `a plain member` for one name, `plain members` for several.
fn plain_members(count: usize) -> &'static str {
    if count == 1 {
        "a plain member"
    } else {
        "plain members"
    }
}

/// What is wrong with a manifest's lists, one sentence each, and what is right with them when
/// nothing is wrong. `all` holds every member with a manifest and `plain` those that carry no
/// `members` label, each with its version.
fn judge(
    manifest: &Manifest,
    all: &BTreeMap<&str, &str>,
    plain: &BTreeMap<&str, &str>,
) -> (RowState, String) {
    let mut problems = Vec::new();
    let mut facts = Vec::new();

    let required = entries(manifest, REQUIRES).unwrap_or_default();
    let requires = resolve("requires", &required, all);
    if !requires.unknown.is_empty() {
        problems.push(format!(
            "requires {}, which is not a member of the set",
            requires.unknown.join(", ")
        ));
    }
    problems.extend(requires.loose);
    problems.extend(requires.mismatched);
    if !required.is_empty() {
        facts.push(format!("requires {}, all members", required.join(", ")));
    }

    if let Some(listed) = entries(manifest, MEMBERS) {
        let lists = resolve("lists", &listed, plain);
        if !lists.unknown.is_empty() {
            problems.push(format!(
                "lists {}, which the set does not hold as {}",
                lists.unknown.join(", "),
                plain_members(lists.unknown.len())
            ));
        }
        problems.extend(lists.loose);
        problems.extend(lists.mismatched);
        if !lists.repeated.is_empty() {
            problems.push(format!(
                "lists {} more than once",
                lists.repeated.join(", ")
            ));
        }
        let missing: Vec<&str> = plain
            .keys()
            .copied()
            .filter(|name| !lists.listed.contains(name))
            .collect();
        if !missing.is_empty() {
            problems.push(format!(
                "does not list {}, which the set holds as {}",
                missing.join(", "),
                plain_members(missing.len())
            ));
        }
        facts.push(if listed.is_empty() {
            "lists no member, exactly the plain members of the set".to_owned()
        } else {
            format!(
                "lists {}, exactly the plain members of the set",
                listed.join(", ")
            )
        });
    }

    if !problems.is_empty() {
        (RowState::RowFailed, problems.join("; "))
    } else if facts.is_empty() {
        (
            RowState::RowPassed,
            "it declares no requirements".to_owned(),
        )
    } else {
        (RowState::RowPassed, facts.join("; "))
    }
}

/// Evaluates the closure of `set`: the rows of [`closure_rows`], each `ROW_PASSED` or
/// `ROW_FAILED`, with what it checked or what is wrong.
pub fn set_closure(set: &ReleaseSet) -> Vec<Row> {
    let mut all = BTreeMap::new();
    let mut plain = BTreeMap::new();
    for manifest in set.members.iter().filter_map(|m| m.manifest.as_ref()) {
        all.entry(manifest.name.as_str())
            .or_insert(manifest.version.as_str());
        if entries(manifest, MEMBERS).is_none() {
            plain
                .entry(manifest.name.as_str())
                .or_insert(manifest.version.as_str());
        }
    }
    set.members
        .iter()
        .enumerate()
        .map(|(at, member)| {
            let name = ExpectedRow::new(subject(at, member), ASSERTION).name();
            let (state, detail) = member.manifest.as_ref().map_or_else(
                || (RowState::RowFailed, "the member has no manifest".to_owned()),
                |manifest| judge(manifest, &all, &plain),
            );
            Row {
                name,
                state: state as i32,
                detail,
            }
        })
        .collect()
}
