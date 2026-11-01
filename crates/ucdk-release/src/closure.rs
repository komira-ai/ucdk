// SPDX-License-Identifier: Apache-2.0
//! The `SET_CLOSURE` validation: a pure function of the set document. It has one derived row
//! per member, `<member>: every requirement names a member of the set`. An image member declares
//! nothing and passes. A FILE member declares through two labels, each a comma-separated list
//! (names are trimmed and an empty name between commas is none):
//!
//! - `requires`: the members it needs. A requirement that names no member of the set is a failed
//!   row naming it.
//! - `members`: the members it brings in. A member with a non-empty `members` label is a list.
//!   At most one member of a set may be a list; two or more are each a failed row naming them
//!   all. With one, its names must be exactly the plain members of the set, the members that have
//!   a manifest and are not a list. A plain member the list leaves out, a name that is not a plain
//!   member, and a name listed twice are each a failed row naming them. A set with no list makes
//!   no claim of this kind, because a set need not have an aggregating member. An empty or blank
//!   `members` label is a failed row, and the member is then not a list and stays a plain member,
//!   so an empty label never exempts a member from the list.
//!
//! Images count as plain members. The set records no kind that tells a library from anything
//! else, and an aggregate that left an image out would not bring it in, so every member is held
//! to be listed. Two members with one name are each a failed row, and a pin against that name is
//! not judged against either of them.
//!
//! An entry of either list is a bare name or an exact pin, `name==version`. A pin holds only
//! when the named member's manifest `version` is, byte for byte, the pinned one: a prefix of it,
//! an extension of it and a member with no version do not match. A pin compares the version and
//! nothing else; the build number and the digest of a member are covered by the set hash. Any
//! other use of a comparison character (`<`, `>`, `=`, `~`, `!`, `*`) is a range, a wildcard or
//! a loose match and is a failed row naming the entry, and only that fault: the name before the
//! character is not also checked against the set.

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

/// The members a list may name, each with its version; `None` for a name two members share,
/// which no pin is judged against.
type Versions<'a> = BTreeMap<&'a str, Option<&'a str>>;

/// Sets the `entries` of a list, introduced by `verb` in its sentences, against `versions`.
fn resolve<'a>(verb: &str, list: &[&'a str], versions: &Versions<'_>) -> Resolved<'a> {
    let mut resolved = Resolved::default();
    for entry in list {
        let (name, pin) = match parse(entry) {
            Entry::Bare(name) => (name, None),
            Entry::Pinned { name, version } => (name, Some(version)),
            Entry::Loose { entry, name } => {
                resolved.loose.push(format!(
                    "{entry} is not a bare name or an exact name==version pin"
                ));
                if !name.is_empty() {
                    resolved.listed.insert(name);
                }
                continue;
            }
        };
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
        if let (Some(want), Some(have)) = (pin, *have)
            && want != have
        {
            let at = if have.is_empty() { "no version" } else { have };
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

/// The members of a set, as `judge` needs to see them.
struct Members<'a> {
    /// Every member with a manifest.
    all: Versions<'a>,
    /// The members with a manifest that are not a list.
    plain: Versions<'a>,
    /// The names of the members that are a list, in the order of the set.
    lists: Vec<&'a str>,
}

/// The entries of a non-empty `members` label: the manifest is a list when this is `Some`.
fn list_of(manifest: &Manifest) -> Option<Vec<&str>> {
    entries(manifest, MEMBERS).filter(|list| !list.is_empty())
}

/// What is wrong with a manifest's lists, one sentence each, and what is right with them when
/// nothing is wrong. `shared` is whether another member has the name of this one.
fn judge(manifest: &Manifest, shared: bool, members: &Members<'_>) -> (RowState, String) {
    let (all, plain) = (&members.all, &members.plain);
    let mut problems = Vec::new();
    let mut facts = Vec::new();

    if shared {
        problems.push("its name is also the name of another member".to_owned());
    }

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

    if entries(manifest, MEMBERS).is_some_and(|list| list.is_empty()) {
        problems.push("its members label is empty; a list names at least one member".to_owned());
    }
    if let Some(listed) = list_of(manifest) {
        if members.lists.len() > 1 {
            problems.push(format!(
                "{} members carry a members list ({}); at most one may",
                members.lists.len(),
                members.lists.join(", ")
            ));
        }
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
        facts.push(format!(
            "lists {}, exactly the plain members of the set",
            listed.join(", ")
        ));
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

/// Records `name` at `version`, or as shared (`None`) when it is already there.
fn place<'a>(versions: &mut Versions<'a>, name: &'a str, version: &'a str) {
    let seen = versions.contains_key(name);
    versions.insert(name, if seen { None } else { Some(version) });
}

/// Evaluates the closure of `set`: the rows of [`closure_rows`], each `ROW_PASSED` or
/// `ROW_FAILED`, with what it checked or what is wrong.
pub fn set_closure(set: &ReleaseSet) -> Vec<Row> {
    let mut members = Members {
        all: BTreeMap::new(),
        plain: BTreeMap::new(),
        lists: Vec::new(),
    };
    for manifest in set.members.iter().filter_map(|m| m.manifest.as_ref()) {
        let (name, version) = (manifest.name.as_str(), manifest.version.as_str());
        let is_list = list_of(manifest).is_some();
        if is_list {
            members.lists.push(name);
        }
        place(&mut members.all, name, version);
        if !is_list {
            place(&mut members.plain, name, version);
        }
    }
    set.members
        .iter()
        .enumerate()
        .map(|(at, member)| {
            let name = ExpectedRow::new(subject(at, member), ASSERTION).name();
            let (state, detail) = member.manifest.as_ref().map_or_else(
                || (RowState::RowFailed, "the member has no manifest".to_owned()),
                |manifest| {
                    let shared = members.all.get(manifest.name.as_str()) == Some(&None);
                    judge(manifest, shared, &members)
                },
            );
            Row {
                name,
                state: state as i32,
                detail,
            }
        })
        .collect()
}
