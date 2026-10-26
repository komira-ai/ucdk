// SPDX-License-Identifier: Apache-2.0
//! The `SET_CLOSURE` validation: a pure function of the set document. It has one derived row
//! per member, `<member>: every requirement names a member of the set`. A member's requirements
//! are the comma-separated names in its `requires` label (names are trimmed and an empty name
//! between commas is none); an image member declares none. A requirement that names no member
//! of the set is a failed row naming it.

use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{Manifest, Member, ReleaseSet, Row, RowState};

use crate::rows::ExpectedRow;

/// The label that lists a member's requirements.
pub const REQUIRES: &str = "requires";

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

/// The names a manifest requires.
fn requirements(manifest: &Manifest) -> Vec<&str> {
    let Some(Metadata::FileMeta(meta)) = &manifest.metadata else {
        return Vec::new();
    };
    meta.labels
        .get(REQUIRES)
        .map(|list| list.split(',').map(str::trim).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|name| !name.is_empty())
        .collect()
}

/// Evaluates the closure of `set`: the rows of [`closure_rows`], each `ROW_PASSED` or
/// `ROW_FAILED`, with the requirements it checked or the ones that name no member.
pub fn set_closure(set: &ReleaseSet) -> Vec<Row> {
    let members: Vec<&str> = set
        .members
        .iter()
        .filter_map(|m| m.manifest.as_ref())
        .map(|m| m.name.as_str())
        .collect();
    set.members
        .iter()
        .enumerate()
        .map(|(at, member)| {
            let name = ExpectedRow::new(subject(at, member), ASSERTION).name();
            let (state, detail) = match &member.manifest {
                None => (RowState::RowFailed, "the member has no manifest".to_owned()),
                Some(manifest) => {
                    let required = requirements(manifest);
                    let missing: Vec<&str> = required
                        .iter()
                        .copied()
                        .filter(|r| !members.contains(r))
                        .collect();
                    if !missing.is_empty() {
                        (
                            RowState::RowFailed,
                            format!(
                                "requires {}, which is not a member of the set",
                                missing.join(", ")
                            ),
                        )
                    } else if required.is_empty() {
                        (
                            RowState::RowPassed,
                            "it declares no requirements".to_owned(),
                        )
                    } else {
                        (
                            RowState::RowPassed,
                            format!("requires {}, all members", required.join(", ")),
                        )
                    }
                }
            };
            Row {
                name,
                state: state as i32,
                detail,
            }
        })
        .collect()
}
