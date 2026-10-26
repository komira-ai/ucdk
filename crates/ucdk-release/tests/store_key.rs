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

#[test]
fn dot_parts_backslash_and_control_characters_are_refused() {
    let refused = [
        "..", ".", "a\\b", "\\", "a\nb", "a\0b", "\t", "a\u{7f}", "\u{85}",
    ];
    for bad in refused {
        for (what, edit) in [
            (
                "stage",
                (|s: &mut SetScope, v: &str| s.stage = v.to_owned()) as fn(&mut SetScope, &str),
            ),
            ("platform", |s, v| s.platform = v.to_owned()),
            ("revision", |s, v| s.revision = v.to_owned()),
            ("generation", |s, v| s.generation = v.to_owned()),
        ] {
            let mut s = scope();
            edit(&mut s, bad);
            for err in [
                StoreKey::member(&s, "tool").expect_err("bad scope"),
                StoreKey::set(&s).expect_err("bad scope"),
                s.prefix().expect_err("bad scope"),
            ] {
                assert_eq!(err.part, what, "{bad:?}");
            }
        }
        let err = StoreKey::member(&scope(), bad).expect_err("bad member name");
        assert_eq!(err.part, "member", "{bad:?}");
    }
}

#[test]
fn a_generation_cannot_climb_out_of_its_directory() {
    let mut s = scope();
    s.generation = "..".to_owned();
    assert!(StoreKey::set(&s).is_err());
}

#[test]
fn names_that_only_look_like_dot_parts_are_kept() {
    let mut s = scope();
    s.revision = "..abc".to_owned();
    s.generation = "a.b".to_owned();
    let key = StoreKey::member(&s, "tool.1").expect("a key");
    assert_eq!(key.as_str(), "build/linux-x86_64/..abc/a.b/m/tool.1");
}
