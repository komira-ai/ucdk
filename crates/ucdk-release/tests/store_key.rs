// SPDX-License-Identifier: Apache-2.0
//! The names a store key is made of: each part names one thing.

use ucdk_release::store::{SetScope, StoreKey};

fn scope() -> SetScope {
    SetScope {
        stage: "build".to_owned(),
        platform: "linux-x86_64".to_owned(),
        revision: "abc".to_owned(),
        generation: "run-1-a1".to_owned(),
    }
}

type Edit = fn(&mut SetScope, &str);

/// One edit per part of a scope, with the name the refusal gives the part.
const PARTS: [(&str, Edit); 4] = [
    ("stage", |s, v| s.stage = v.to_owned()),
    ("platform", |s, v| s.platform = v.to_owned()),
    ("revision", |s, v| s.revision = v.to_owned()),
    ("generation", |s, v| s.generation = v.to_owned()),
];

#[test]
fn member_and_set_keys_carry_the_whole_scope() {
    let member = StoreKey::member(&scope(), "tool").expect("a key");
    assert_eq!(member.as_str(), "build/linux-x86_64/abc/run-1-a1/m/tool");
    assert_eq!(member.to_string(), member.as_str());
    let set = StoreKey::set(&scope()).expect("a key");
    assert_eq!(set.as_str(), "build/linux-x86_64/abc/run-1-a1/set");
    assert_eq!(
        scope().prefix().expect("prefix"),
        "build/linux-x86_64/abc/run-1-a1/"
    );
    // A scope that differs in any one part has no key in common with this one.
    for (_, edit) in PARTS {
        let mut other = scope();
        edit(&mut other, "other");
        assert_ne!(StoreKey::member(&other, "tool").expect("a key"), member);
        assert_ne!(StoreKey::set(&other).expect("a key"), set);
    }
}

#[test]
fn dot_parts_backslash_and_control_characters_are_refused() {
    // Every part, whatever constructor reaches it, is held to the one rule: not empty, not `.`
    // or `..`, and no `/`, `\` or control character.
    let refused = [
        "", "..", ".", "/", "a/b", "/a", "a\\b", "\\", "a\nb", "a\0b", "\t", "a\u{7f}", "\u{85}",
    ];
    for bad in refused {
        for (what, edit) in PARTS {
            let mut s = scope();
            edit(&mut s, bad);
            for err in [
                StoreKey::member(&s, "tool").expect_err("bad scope"),
                StoreKey::set(&s).expect_err("bad scope"),
                s.prefix().expect_err("bad scope"),
            ] {
                assert_eq!(err.part, what, "{bad:?}");
                assert!(err.to_string().contains(what), "{err}");
            }
        }
        let err = StoreKey::member(&scope(), bad).expect_err("bad member name");
        assert_eq!(err.part, "member", "{bad:?}");
        assert!(err.to_string().contains("member"), "{err}");
    }
}

#[test]
fn the_first_part_that_is_refused_is_named() {
    let mut s = scope();
    s.revision = "..".to_owned();
    s.generation = "/".to_owned();
    assert_eq!(StoreKey::set(&s).expect_err("bad").part, "revision");
    // The member name is read after the scope.
    assert_eq!(StoreKey::member(&s, "").expect_err("bad").part, "revision");
    assert_eq!(
        StoreKey::member(&scope(), "").expect_err("bad").part,
        "member"
    );
}

#[test]
fn a_scope_part_that_could_escape_is_refused() {
    // A part that is a parent, a root or a path of several parts would let a filesystem store
    // resolve a key outside its generation.
    for (what, edit) in PARTS {
        for escape in ["..", "../x", "/etc", "x/../y", "..\\x"] {
            let mut s = scope();
            edit(&mut s, escape);
            assert_eq!(StoreKey::set(&s).expect_err("escapes").part, what);
            assert_eq!(
                StoreKey::member(&s, "tool").expect_err("escapes").part,
                what
            );
        }
    }
    for escape in ["..", "../x", "x/y"] {
        assert_eq!(
            StoreKey::member(&scope(), escape)
                .expect_err("escapes")
                .part,
            "member"
        );
    }
}

#[test]
fn names_that_only_look_like_dot_parts_are_kept() {
    let mut s = scope();
    s.revision = "..abc".to_owned();
    s.generation = "a.b".to_owned();
    let key = StoreKey::member(&s, "tool.1").expect("a key");
    assert_eq!(key.as_str(), "build/linux-x86_64/..abc/a.b/m/tool.1");
}
