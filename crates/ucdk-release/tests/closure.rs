// SPDX-License-Identifier: Apache-2.0
//! The `SET_CLOSURE` validation: one derived row per member, a requirement that names no member
//! is a failed row, a pinned requirement must match the pinned version exactly, and the members
//! a member lists must be exactly the plain members of the set.

use std::collections::BTreeMap;

use ucdk_release::closure::{ASSERTION, MEMBERS, REQUIRES, closure_rows, set_closure};
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

/// A FILE member at `version` with the given labels.
fn labelled(name: &str, version: &str, labels: &[(&str, &str)]) -> Member {
    let mut member = file(name, None);
    let manifest = member.manifest.as_mut().expect("a manifest");
    manifest.version = version.to_owned();
    manifest.metadata = Some(Metadata::FileMeta(FileMeta {
        labels: labels
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }));
    member
}

fn leaf(name: &str, version: &str) -> Member {
    labelled(name, version, &[])
}

fn requiring(name: &str, requires: &str) -> Member {
    labelled(name, "1.0.0", &[(REQUIRES, requires)])
}

/// A member that lists the members it brings in.
fn listing(name: &str, members: &str) -> Member {
    labelled(name, "1.0.0", &[(MEMBERS, members)])
}

fn detail_of(members: Vec<Member>, at: usize) -> (RowState, String) {
    let rows = set_closure(&set(members));
    (state(&rows[at]), rows[at].detail.clone())
}

#[test]
fn an_exact_pin_passes_when_the_member_has_that_version() {
    let pinned = set(vec![
        requiring("app", "lib==1.2.3, tool == 4.5.6, other"),
        leaf("lib", "1.2.3"),
        leaf("tool", "4.5.6"),
        leaf("other", "9"),
    ]);
    let rows = set_closure(&pinned);
    assert_eq!(state(&rows[0]), RowState::RowPassed);
    assert_eq!(
        rows[0].detail,
        "requires lib==1.2.3, tool == 4.5.6, other, all members"
    );
    assert_eq!(
        account_rows(&closure_rows(&pinned), &rows),
        Ok(Verdict::Succeeded)
    );
}

#[test]
fn a_pin_to_another_version_or_a_prefix_of_it_is_a_failed_row() {
    for (pin, have) in [
        ("lib==1.2.4", "1.2.3"),
        ("lib==1.2", "1.2.3"),
        ("lib==1.2.3", "1.2"),
        ("lib==1.2.3", "1.2.30"),
        ("lib==v1.2.3", "1.2.3"),
        ("lib==1.2.3", "1.2.3-rc1"),
        ("lib==1.2.3", "1.2.3 "),
    ] {
        let (verdict, detail) = detail_of(vec![requiring("app", pin), leaf("lib", have)], 0);
        assert_eq!(verdict, RowState::RowFailed, "{pin} against {have:?}");
        let want = pin.strip_prefix("lib==").expect("a pin");
        assert_eq!(
            detail,
            format!("requires lib=={want}, but lib is at {have}"),
            "{pin} against {have:?}"
        );
    }
    // A member with no version is at no version, and no pin matches it.
    let (verdict, detail) = detail_of(vec![requiring("app", "lib==1"), file("lib", None)], 0);
    assert_eq!(verdict, RowState::RowFailed);
    assert_eq!(detail, "requires lib==1, but lib is at no version");
}

#[test]
fn a_range_a_wildcard_or_a_loose_match_is_not_an_exact_pin() {
    for loose in [
        "lib>=1.2.3",
        "lib<=1.2.3",
        "lib>1",
        "lib<2",
        "lib~=1.2",
        "lib!=1.2.3",
        "lib=1.2.3",
        "lib===1.2.3",
        "lib==1.2.*",
        "lib==*",
        "lib==>=1.2.3",
        "lib==",
        "lib== ",
        "lib==1.2.3 4",
        "==1.2.3",
        "lib*",
    ] {
        let (verdict, detail) = detail_of(vec![requiring("app", loose), leaf("lib", "1.2.3")], 0);
        assert_eq!(verdict, RowState::RowFailed, "{loose}");
        assert_eq!(
            detail,
            format!(
                "{} is not a bare name or an exact name==version pin",
                loose.trim()
            ),
            "{loose}"
        );
    }
}

#[test]
fn a_pin_naming_a_non_member_is_reported_with_the_other_faults() {
    let (verdict, detail) = detail_of(
        vec![
            requiring("app", "ghost==1, lib==2, lib>=1, phantom"),
            leaf("lib", "1"),
        ],
        0,
    );
    assert_eq!(verdict, RowState::RowFailed);
    assert_eq!(
        detail,
        "requires ghost, phantom, which is not a member of the set; \
         lib>=1 is not a bare name or an exact name==version pin; \
         requires lib==2, but lib is at 1"
    );
}

#[test]
fn a_list_equal_to_the_plain_members_passes() {
    let bundle = set(vec![
        listing("bundle", "lib-a, lib-b"),
        leaf("lib-a", "1"),
        leaf("lib-b", "2"),
    ]);
    let rows = set_closure(&bundle);
    assert!(rows.iter().all(|r| state(r) == RowState::RowPassed));
    assert_eq!(
        rows[0].detail,
        "lists lib-a, lib-b, exactly the plain members of the set"
    );
    assert_eq!(rows[1].detail, "it declares no requirements");
    // The order of the list is not the order of the set.
    let reordered = set(vec![
        listing("bundle", "lib-b,lib-a"),
        leaf("lib-a", "1"),
        leaf("lib-b", "2"),
    ]);
    assert_eq!(state(&set_closure(&reordered)[0]), RowState::RowPassed);
    assert_eq!(
        account_rows(&closure_rows(&bundle), &rows),
        Ok(Verdict::Succeeded)
    );
}

#[test]
fn a_plain_member_the_list_leaves_out_is_a_failed_row() {
    let (verdict, detail) = detail_of(
        vec![
            listing("bundle", "lib-a"),
            leaf("lib-a", "1"),
            leaf("lib-b", "2"),
            leaf("lib-c", "3"),
        ],
        0,
    );
    assert_eq!(verdict, RowState::RowFailed);
    assert_eq!(
        detail,
        "does not list lib-b, lib-c, which the set holds as plain members"
    );
    // An empty list leaves out every one.
    let (verdict, detail) = detail_of(vec![listing("bundle", ""), leaf("lib-a", "1")], 0);
    assert_eq!(verdict, RowState::RowFailed);
    assert_eq!(
        detail,
        "does not list lib-a, which the set holds as a plain member"
    );
}

#[test]
fn a_name_the_list_adds_beyond_the_plain_members_is_a_failed_row() {
    let (verdict, detail) = detail_of(
        vec![
            listing("bundle", "lib-a, ghost, other-bundle, bundle, ghost"),
            leaf("lib-a", "1"),
            listing("other-bundle", "lib-a"),
        ],
        0,
    );
    assert_eq!(verdict, RowState::RowFailed);
    assert_eq!(
        detail,
        "lists ghost, other-bundle, bundle, which the set does not hold as plain members; \
         lists ghost more than once"
    );
}

#[test]
fn a_name_listed_twice_is_a_failed_row_even_when_the_set_is_otherwise_equal() {
    let (verdict, detail) = detail_of(
        vec![
            listing("bundle", "lib-a, lib-b, lib-a"),
            leaf("lib-a", "1"),
            leaf("lib-b", "2"),
        ],
        0,
    );
    assert_eq!(verdict, RowState::RowFailed);
    assert_eq!(detail, "lists lib-a more than once");
}

#[test]
fn a_list_pins_its_members_exactly_too() {
    let good = set(vec![
        listing("bundle", "lib-a==1, lib-b==2"),
        leaf("lib-a", "1"),
        leaf("lib-b", "2"),
    ]);
    let rows = set_closure(&good);
    assert_eq!(state(&rows[0]), RowState::RowPassed);
    assert_eq!(
        rows[0].detail,
        "lists lib-a==1, lib-b==2, exactly the plain members of the set"
    );
    let (verdict, detail) = detail_of(
        vec![
            listing("bundle", "lib-a==1.0, lib-b>=2"),
            leaf("lib-a", "1"),
            leaf("lib-b", "2"),
        ],
        0,
    );
    assert_eq!(verdict, RowState::RowFailed);
    assert_eq!(
        detail,
        "lib-b>=2 is not a bare name or an exact name==version pin; \
         lists lib-a==1.0, but lib-a is at 1"
    );
    // A pin to a name that is not a plain member is an extra name, not a version fault.
    let (_, detail) = detail_of(vec![listing("bundle", "ghost==1")], 0);
    assert_eq!(
        detail,
        "lists ghost, which the set does not hold as a plain member"
    );
}

#[test]
fn the_plain_members_are_those_with_a_manifest_and_no_list_of_their_own() {
    // Two lists in one set each name the plain members and neither names the other; a member
    // with no manifest and an image member (which has no labels) are held to the same rule.
    let two = set(vec![
        listing("bundle-a", "lib, image"),
        listing("bundle-b", "image,lib"),
        leaf("lib", "1"),
        image("image"),
        Member { manifest: None },
    ]);
    let rows = set_closure(&two);
    assert_eq!(state(&rows[0]), RowState::RowPassed);
    assert_eq!(state(&rows[1]), RowState::RowPassed);
    assert_eq!(state(&rows[4]), RowState::RowFailed);
    // A list that names the other list is naming a name that is not a plain member.
    let (verdict, detail) = detail_of(
        vec![
            listing("bundle-a", "lib, bundle-b"),
            listing("bundle-b", "lib"),
            leaf("lib", "1"),
        ],
        0,
    );
    assert_eq!(verdict, RowState::RowFailed);
    assert_eq!(
        detail,
        "lists bundle-b, which the set does not hold as a plain member"
    );
    // A list alone, with an empty list, is equal to the empty set of plain members.
    let (verdict, detail) = detail_of(vec![listing("bundle", " , ")], 0);
    assert_eq!(verdict, RowState::RowPassed);
    assert_eq!(
        detail,
        "lists no member, exactly the plain members of the set"
    );
}

#[test]
fn a_member_may_require_and_list_and_both_are_held() {
    let both = |requires: &str, members: &str| {
        let bundle = labelled(
            "bundle",
            "1.0.0",
            &[(REQUIRES, requires), (MEMBERS, members)],
        );
        vec![bundle, leaf("lib", "1"), leaf("base", "7")]
    };
    // `base` is a plain member too, so the list must name it as well as `lib`.
    let (verdict, detail) = detail_of(both("base==7", "base, lib"), 0);
    assert_eq!(verdict, RowState::RowPassed);
    assert_eq!(
        detail,
        "requires base==7, all members; lists base, lib, exactly the plain members of the set"
    );
    let (verdict, detail) = detail_of(both("base==8", "lib"), 0);
    assert_eq!(verdict, RowState::RowFailed);
    assert_eq!(
        detail,
        "requires base==8, but base is at 7; does not list base, which the set holds as a plain member"
    );
}
