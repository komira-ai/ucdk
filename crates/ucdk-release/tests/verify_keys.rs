// SPDX-License-Identifier: Apache-2.0
//! Verifying the keys of a generation, and the names of keys.

use ucdk_release::store::{SetScope, StoreKey};
use ucdk_release::verify::{Check, Expect, verify_keys};
use ucdk_release_proto::v1::{Manifest, Member, Reason, ReleaseSet};

fn scope() -> SetScope {
    SetScope {
        stage: "build".to_owned(),
        platform: "linux-x86_64".to_owned(),
        revision: "abc".to_owned(),
        generation: "run-1-a1".to_owned(),
    }
}

fn set(names: &[&str]) -> ReleaseSet {
    ReleaseSet {
        members: names
            .iter()
            .map(|n| Member {
                manifest: Some(Manifest {
                    name: (*n).to_owned(),
                    ..Manifest::default()
                }),
            })
            .collect(),
        ..ReleaseSet::default()
    }
}

fn member(name: &str) -> StoreKey {
    StoreKey::member(&scope(), name).expect("a key")
}

fn set_key() -> StoreKey {
    StoreKey::set(&scope()).expect("a key")
}

fn raw(key: &str) -> StoreKey {
    // A key of another scope is made through its own scope, since a key is made only by the
    // constructors; this spells the scope the key belongs to.
    let (head, name) = key.rsplit_once('/').expect("a key with a name");
    let parts: Vec<&str> = head.split('/').collect();
    let other = SetScope {
        stage: parts[0].to_owned(),
        platform: parts[1].to_owned(),
        revision: parts[2].to_owned(),
        generation: parts[3].to_owned(),
    };
    StoreKey::member(&other, name).expect("a key")
}

#[test]
fn a_key_is_the_scope_and_the_name() {
    assert_eq!(
        member("tool").as_str(),
        "build/linux-x86_64/abc/run-1-a1/m/tool"
    );
    assert_eq!(set_key().as_str(), "build/linux-x86_64/abc/run-1-a1/set");
    assert_eq!(
        scope().prefix().expect("prefix"),
        "build/linux-x86_64/abc/run-1-a1/"
    );
    assert_eq!(member("tool").to_string(), member("tool").as_str());
}

#[test]
fn a_key_part_that_is_empty_or_holds_a_slash_is_refused() {
    for (what, edit) in [
        (
            "stage",
            (|s: &mut SetScope| s.stage.clear()) as fn(&mut SetScope),
        ),
        ("platform", |s| s.platform = "a/b".to_owned()),
        ("revision", |s| s.revision.clear()),
        ("generation", |s| s.generation = "/".to_owned()),
    ] {
        let mut s = scope();
        edit(&mut s);
        for err in [
            StoreKey::member(&s, "tool").expect_err("bad scope"),
            StoreKey::set(&s).expect_err("bad scope"),
            s.prefix().expect_err("bad scope"),
        ] {
            assert_eq!(err.part, what);
            assert!(err.to_string().contains(what), "{err}");
        }
    }
    for name in ["", "a/b"] {
        let err = StoreKey::member(&scope(), name).expect_err("bad name");
        assert_eq!(err.part, "member");
    }
}

#[test]
fn before_the_set_document_exists_only_member_keys_pass() {
    let keys = [member("tool"), member("image")];
    assert_eq!(
        verify_keys(
            &scope(),
            &set(&["image", "tool"]),
            Expect::MembersOnly,
            &keys
        ),
        Ok(())
    );
    // The set key is neither required nor refused before the document is written.
    let with_set = [member("tool"), member("image"), set_key()];
    assert_eq!(
        verify_keys(
            &scope(),
            &set(&["image", "tool"]),
            Expect::MembersOnly,
            &with_set
        ),
        Ok(())
    );
}

#[test]
fn after_the_write_a_missing_set_document_is_refused() {
    let keys = [member("tool"), member("image")];
    let err = verify_keys(
        &scope(),
        &set(&["image", "tool"]),
        Expect::MembersAndSet,
        &keys,
    )
    .expect_err("no set document");
    assert_eq!((err.check, err.reason), (Check::Keys, Reason::SetMember));
    assert!(err.to_string().contains("/set"), "{err}");
    let with_set = [member("tool"), member("image"), set_key()];
    assert_eq!(
        verify_keys(
            &scope(),
            &set(&["image", "tool"]),
            Expect::MembersAndSet,
            &with_set
        ),
        Ok(())
    );
}

#[test]
fn an_extra_key_is_set_member() {
    let names = set(&["tool"]);
    for expect in [Expect::MembersOnly, Expect::MembersAndSet] {
        let keys = [member("tool"), member("stray"), set_key()];
        let err = verify_keys(&scope(), &names, expect, &keys).expect_err("an extra key");
        assert_eq!((err.check, err.reason), (Check::Keys, Reason::SetMember));
        assert!(err.to_string().contains("stray"), "{err}");
    }
}

#[test]
fn a_member_with_no_key_is_refused() {
    let err = verify_keys(
        &scope(),
        &set(&["tool", "image"]),
        Expect::MembersOnly,
        &[member("tool")],
    )
    .expect_err("a missing member");
    assert_eq!(err.reason, Reason::SetMember);
    assert!(err.to_string().contains("image"), "{err}");
}

#[test]
fn the_keys_of_other_generations_and_scopes_are_not_this_generations() {
    let keys = [
        member("tool"),
        raw("build/linux-x86_64/abc/run-1-a10/m/stray"),
        raw("build/linux-x86_64/abc/run-1-a2/m/stray"),
        raw("build/linux-aarch64/abc/run-1-a1/m/stray"),
        raw("check/linux-x86_64/abc/run-1-a1/m/stray"),
    ];
    assert_eq!(
        verify_keys(&scope(), &set(&["tool"]), Expect::MembersOnly, &keys),
        Ok(())
    );
}

#[test]
fn a_scope_or_a_member_name_that_cannot_make_a_key_is_refused() {
    let mut bad = scope();
    bad.stage.clear();
    let err = verify_keys(&bad, &set(&["tool"]), Expect::MembersOnly, &[]).expect_err("bad scope");
    assert_eq!(err.check, Check::Keys);
    let err =
        verify_keys(&scope(), &set(&["a/b"]), Expect::MembersOnly, &[]).expect_err("bad name");
    assert_eq!(err.check, Check::Keys);
    // A member with no manifest has no name to make a key of.
    let nameless = ReleaseSet {
        members: vec![Member { manifest: None }],
        ..ReleaseSet::default()
    };
    assert!(verify_keys(&scope(), &nameless, Expect::MembersOnly, &[]).is_err());
}
