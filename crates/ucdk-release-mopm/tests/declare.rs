// SPDX-License-Identifier: Apache-2.0
//! The names a release declared: `stage` sends nothing for a set that holds any other name, a
//! registry never told any names refuses every stage, and `abort` is not refused for a name.

mod common;

use common::*;
use ucdk_core::ManualClock;
use ucdk_release_mopm::{Method, Registry};
use ucdk_release_seams::{DeclareError, SeamError, StageAnswer};

fn clock() -> ManualClock {
    ManualClock::at(1_000_000)
}

/// A registry that answers a stage of `alpha` as a new group whose files are all stored.
fn held(id: &str) -> Fake {
    let (_, members, _) = world();
    Fake::default()
        .on(
            Method::Get,
            &group_path(id),
            vec![error_reply(404, "group_not_found")],
        )
        .on(
            Method::Post,
            GROUPS,
            vec![json_reply(
                201,
                &created_json(id, "open", &members, &["linux-64", "noarch"]),
            )],
        )
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| (*n).to_owned()).collect()
}

#[test]
fn a_registry_never_told_any_names_refuses_every_stage_naming_the_member_and_sends_nothing() {
    let (owned, members, id) = world();
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let mut reg = Registry::new(held(&id), group_config(), Box::new(clock()));
    let result = reg.stage(&caller(), &members, &files);
    assert_eq!(
        result,
        Err(SeamError::Undeclared {
            name: "alpha".to_owned()
        })
    );
    assert!(reg.transport().log.is_empty());
    // An empty declaration declares nothing, and refuses the same.
    reg.declare(&[]).unwrap();
    let result = reg.stage(&caller(), &members, &files);
    assert_eq!(stage_kind(&result), "undeclared");
    assert!(reg.transport().log.is_empty());
}

#[test]
fn a_set_with_one_undeclared_member_is_refused_whole_and_names_that_member() {
    let mut owned = alpha();
    owned.push(Owned::new("beta", "2.0.0", "any"));
    let members = set(&owned);
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let mut reg = registry(Fake::default(), &clock());
    let result = reg.stage(&caller(), &members, &files);
    assert_eq!(
        result,
        Err(SeamError::Undeclared {
            name: "beta".to_owned()
        })
    );
    assert!(reg.transport().log.is_empty());
}

#[test]
fn a_declared_name_matches_ignoring_ascii_case_and_the_stage_goes_on() {
    let (owned, members, id) = world();
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let mut reg = Registry::new(held(&id), group_config(), Box::new(clock()));
    // A member name is lower case by mopm's grammar (`members` refuses any other), so the case
    // that can differ is the declaration's.
    reg.declare(&names(&["ALPHA"])).unwrap();
    assert_eq!(
        reg.stage(&caller(), &members, &files),
        Ok(StageAnswer::Duplicate)
    );
    assert_eq!(
        reg.transport().lines(),
        [format!("GET {}", group_path(&id)), format!("POST {GROUPS}")]
    );
}

#[test]
fn a_second_declaration_replaces_the_first_and_a_refused_one_changes_nothing() {
    let (owned, members, id) = world();
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let mut reg = registry(held(&id), &clock());
    // Replaced by a declaration without `alpha`: the stage is refused.
    reg.declare(&names(&["beta"])).unwrap();
    assert_eq!(
        stage_kind(&reg.stage(&caller(), &members, &files)),
        "undeclared"
    );
    assert!(reg.transport().log.is_empty());
    // Each refused declaration names its entry and leaves `beta` declared, so `alpha` stays
    // refused.
    let refused = [
        (names(&["alpha", ""]), DeclareError::Empty { index: 1 }),
        (
            names(&["alpha", "al pha"]),
            DeclareError::Whitespace {
                name: "al pha".to_owned(),
            },
        ),
        (
            names(&["alpha", "a/b"]),
            DeclareError::Slash {
                name: "a/b".to_owned(),
            },
        ),
        (
            names(&["alpha", "Alpha"]),
            DeclareError::Duplicate {
                name: "Alpha".to_owned(),
            },
        ),
    ];
    for (declaration, error) in refused {
        assert_eq!(reg.declare(&declaration), Err(error));
        assert_eq!(
            stage_kind(&reg.stage(&caller(), &members, &files)),
            "undeclared"
        );
        assert!(reg.transport().log.is_empty());
    }
    // Declared again, the same set stages.
    reg.declare(&names(&["beta", "alpha"])).unwrap();
    assert_eq!(
        reg.stage(&caller(), &members, &files),
        Ok(StageAnswer::Duplicate)
    );
}

#[test]
fn an_abort_is_not_refused_for_a_name_the_release_did_not_declare() {
    let (_, members, id) = world();
    let aborted = group_json(&id, "aborted", &members, &[]);
    let fake = Fake::default().on(
        Method::Delete,
        &group_path(&id),
        vec![json_reply(200, &aborted)],
    );
    let mut reg = Registry::new(fake, group_config(), Box::new(clock()));
    assert_eq!(reg.abort(&caller(), &members), Ok(()));
    assert_eq!(
        reg.transport().lines(),
        [format!("DELETE {}", group_path(&id))]
    );
}
