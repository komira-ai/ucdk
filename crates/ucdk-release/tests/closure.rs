// SPDX-License-Identifier: Apache-2.0
//! The `SET_CLOSURE` validation: one derived row per member, and a requirement that names no
//! member is a failed row.

use std::collections::BTreeMap;

use ucdk_release::closure::{ASSERTION, REQUIRES, closure_rows, set_closure};
use ucdk_release::rows::account_rows;
use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{
    ArtifactType, FileMeta, Manifest, Member, OciMeta, ReleaseSet, Row, RowState, Verdict,
};

fn file(name: &str, requires: Option<&str>) -> Member {
    let labels: BTreeMap<String, String> = requires
        .map(|r| (REQUIRES.to_owned(), r.to_owned()))
        .into_iter()
        .collect();
    Member {
        manifest: Some(Manifest {
            r#type: ArtifactType::File as i32,
            name: name.to_owned(),
            metadata: Some(Metadata::FileMeta(FileMeta { labels })),
            ..Manifest::default()
        }),
    }
}

fn image(name: &str) -> Member {
    Member {
        manifest: Some(Manifest {
            r#type: ArtifactType::Oci as i32,
            name: name.to_owned(),
            metadata: Some(Metadata::Oci(OciMeta {
                os: "linux".to_owned(),
                cpu: "x86_64".to_owned(),
            })),
            ..Manifest::default()
        }),
    }
}

fn set(members: Vec<Member>) -> ReleaseSet {
    ReleaseSet {
        members,
        ..ReleaseSet::default()
    }
}

fn state(row: &Row) -> RowState {
    RowState::try_from(row.state).expect("a known state")
}

#[test]
fn a_closed_set_passes_with_one_row_per_member() {
    let closed = set(vec![
        file("app", Some("lib-a, lib-b")),
        file("lib-a", Some("lib-b")),
        file("lib-b", None),
        file("tool", Some("")),
    ]);
    let rows = set_closure(&closed);
    assert_eq!(rows.len(), 4);
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "app: every requirement names a member of the set",
            "lib-a: every requirement names a member of the set",
            "lib-b: every requirement names a member of the set",
            "tool: every requirement names a member of the set",
        ]
    );
    assert!(rows.iter().all(|r| state(r) == RowState::RowPassed));
    assert_eq!(rows[0].detail, "requires lib-a, lib-b, all members");
    assert_eq!(rows[2].detail, "it declares no requirements");
    // The rows are the derived ones, in order, so the accounting accepts them.
    assert_eq!(
        account_rows(&closure_rows(&closed), &rows),
        Ok(Verdict::Succeeded)
    );
    assert_eq!(ASSERTION, "every requirement names a member of the set");
}

#[test]
fn a_requirement_naming_a_non_member_is_a_failed_row() {
    let open = set(vec![
        file("app", Some("lib, ghost , phantom,,")),
        file("lib", None),
    ]);
    let rows = set_closure(&open);
    assert_eq!(state(&rows[0]), RowState::RowFailed);
    assert_eq!(
        rows[0].detail,
        "requires ghost, phantom, which is not a member of the set"
    );
    // The member that is fine still passes: one row each, and only the open one fails.
    assert_eq!(state(&rows[1]), RowState::RowPassed);
    assert_eq!(
        account_rows(&closure_rows(&open), &rows),
        Ok(Verdict::ValidationFailed)
    );
    // A requirement is a whole name: a prefix of a member is not that member.
    let prefix = set(vec![file("app", Some("li")), file("lib", None)]);
    assert_eq!(state(&set_closure(&prefix)[0]), RowState::RowFailed);
    // A member requiring itself is a member.
    let itself = set(vec![file("app", Some("app"))]);
    assert_eq!(state(&set_closure(&itself)[0]), RowState::RowPassed);
}

#[test]
fn an_oci_member_declares_no_requirements_and_passes() {
    let with_image = set(vec![file("app", Some("image")), image("image")]);
    let rows = set_closure(&with_image);
    assert_eq!(state(&rows[0]), RowState::RowPassed);
    assert_eq!(
        rows[1].name,
        "image: every requirement names a member of the set"
    );
    assert_eq!(state(&rows[1]), RowState::RowPassed);
    assert_eq!(rows[1].detail, "it declares no requirements");
    // A manifest with no metadata declares none either.
    let bare = Member {
        manifest: Some(Manifest {
            name: "bare".to_owned(),
            ..Manifest::default()
        }),
    };
    assert_eq!(
        state(&set_closure(&set(vec![bare]))[0]),
        RowState::RowPassed
    );
}

#[test]
fn a_member_without_a_manifest_is_a_failed_row() {
    let broken = set(vec![file("a", None), Member { manifest: None }]);
    let rows = set_closure(&broken);
    assert_eq!(
        rows[1].name,
        "member 1: every requirement names a member of the set"
    );
    assert_eq!(state(&rows[1]), RowState::RowFailed);
    assert_eq!(rows[1].detail, "the member has no manifest");
    assert_eq!(
        account_rows(&closure_rows(&broken), &rows),
        Ok(Verdict::ValidationFailed)
    );
}

#[test]
fn an_empty_set_derives_no_row() {
    let empty = set(Vec::new());
    assert!(closure_rows(&empty).is_empty());
    assert!(set_closure(&empty).is_empty());
}
