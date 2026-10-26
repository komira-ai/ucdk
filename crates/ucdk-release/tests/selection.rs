// SPDX-License-Identifier: Apache-2.0
//! Selection: what a run is asked to do, and what a selector that matches nothing says.

mod common;

use common::*;
use ucdk_release::selection::{Holder, Item, Selection, SelectionError, Selector};
use ucdk_release_proto::v1::{Reason, Stage, Trigger, ValidationKind};

fn texts(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

fn select(items: &[&str]) -> Selection {
    Selection::parse(&texts(items)).expect("selectors")
}

/// A stage of a Build action with a closure and a Test action.
fn building() -> Stage {
    stage(
        "build",
        Trigger::Release,
        &[],
        vec![
            build_action(
                "build",
                "any",
                &["tool"],
                vec![validation("closure", ValidationKind::SetClosure, 0)],
            ),
            test_action("test", &["//release:set_test"]),
        ],
    )
}

/// A stage of a Publish action with a preflight closure, a Smoke action with a read-back, and
/// a read-back of its own.
fn publishing() -> Stage {
    let mut publish = publish_action("publish", "any", &["tool"], "stable");
    publish.validations = vec![validation("preflight", ValidationKind::SetClosure, 0)];
    Stage {
        validations: vec![validation("seen", ValidationKind::ChannelReadback, 0)],
        ..stage(
            "publish",
            Trigger::Release,
            &["build"],
            vec![
                publish,
                smoke_action(
                    "smoke",
                    "any",
                    &["tool"],
                    "stable",
                    vec![validation("visible", ValidationKind::ChannelReadback, 0)],
                ),
            ],
        )
    }
}

fn action(name: &str, validations: bool) -> Item {
    Item::Action {
        action: name.to_owned(),
        validations,
    }
}

fn on_action(holder: &str, name: &str) -> Item {
    Item::Validation {
        holder: Holder::Action(holder.to_owned()),
        name: name.to_owned(),
    }
}

fn on_stage(name: &str) -> Item {
    Item::Validation {
        holder: Holder::Stage,
        name: name.to_owned(),
    }
}

#[test]
fn any_selector_is_selective() {
    assert!(!Selection::default().is_selective());
    assert!(!select(&[]).is_selective());
    assert!(select(&["action:build"]).is_selective());
    assert!(select(&["validation:closure"]).is_selective());
}

#[test]
fn a_selector_is_a_kind_and_a_name() {
    for bad in [
        "",
        "build",
        "action",
        "action:",
        "validation:",
        "stage:build",
        "Action:build",
        "step:build",
    ] {
        let err = Selection::parse(&texts(&[bad])).expect_err(bad);
        assert_eq!(err, SelectionError::Malformed(bad.to_owned()));
        assert_eq!(err.reason(), Reason::Selector);
        assert!(err.to_string().contains("action:<name>"), "{err}");
    }
}

#[test]
fn selectors_are_sorted_bytewise_and_each_counts_once() {
    let selection = select(&["validation:b", "action:z", "action:a", "validation:b"]);
    assert_eq!(selection.texts(), ["action:a", "action:z", "validation:b"]);
    assert_eq!(Selector::Action("x".to_owned()).to_string(), "action:x");
    assert_eq!(
        Selector::Validation("x".to_owned()).to_string(),
        "validation:x"
    );
}

#[test]
fn no_selector_runs_every_action_with_its_validations() {
    assert_eq!(
        Selection::default().resolve(&building()).expect("resolved"),
        [action("build", true), action("test", true)]
    );
    // The stage's own validations run after the last action.
    assert_eq!(
        Selection::default()
            .resolve(&publishing())
            .expect("resolved"),
        [
            action("publish", true),
            action("smoke", true),
            on_stage("seen")
        ]
    );
}

#[test]
fn an_action_selector_runs_a_build_action_without_its_validations() {
    assert_eq!(
        select(&["action:build"])
            .resolve(&building())
            .expect("resolved"),
        [action("build", false)]
    );
    // A Smoke action's read-back is not run with it either.
    assert_eq!(
        select(&["action:smoke"])
            .resolve(&publishing())
            .expect("resolved"),
        [action("smoke", false)]
    );
}

#[test]
fn a_publish_action_selector_keeps_its_pre_commit_validations() {
    assert_eq!(
        select(&["action:publish"])
            .resolve(&publishing())
            .expect("resolved"),
        [action("publish", true)]
    );
    // Naming the validation as well does not run it twice.
    assert_eq!(
        select(&["action:publish", "validation:preflight"])
            .resolve(&publishing())
            .expect("resolved"),
        [action("publish", true)]
    );
}

#[test]
fn a_validation_selector_runs_that_validation_alone() {
    assert_eq!(
        select(&["validation:closure"])
            .resolve(&building())
            .expect("resolved"),
        [on_action("build", "closure")]
    );
    assert_eq!(
        select(&["validation:preflight"])
            .resolve(&publishing())
            .expect("resolved"),
        [on_action("publish", "preflight")]
    );
    assert_eq!(
        select(&["validation:seen"])
            .resolve(&publishing())
            .expect("resolved"),
        [on_stage("seen")]
    );
}

#[test]
fn selected_parts_run_in_the_order_of_the_stage() {
    assert_eq!(
        select(&[
            "validation:seen",
            "action:test",
            "validation:visible",
            "action:build"
        ])
        .resolve(&{
            let mut both = building();
            both.actions.extend(publishing().actions);
            both.validations = publishing().validations;
            both
        })
        .expect("resolved"),
        [
            action("build", false),
            action("test", false),
            on_action("smoke", "visible"),
            on_stage("seen")
        ]
    );
}

#[test]
fn no_match_lists_choices() {
    let err = select(&["action:three"])
        .resolve(&building())
        .expect_err("matches nothing");
    assert_eq!(err.reason(), Reason::SelectorNoMatch);
    let SelectionError::NoMatch { selector, choices } = &err else {
        panic!("a no-match error");
    };
    assert_eq!(selector, &Selector::Action("three".to_owned()));
    assert_eq!(
        choices,
        &[
            Selector::Action("build".to_owned()),
            Selector::Action("test".to_owned()),
            Selector::Validation("closure".to_owned()),
        ]
    );
    let text = err.to_string();
    for wanted in [
        "action:three",
        "action:build",
        "action:test",
        "validation:closure",
    ] {
        assert!(text.contains(wanted), "{text}");
    }
}

#[test]
fn a_selector_that_matches_nothing_refuses_even_beside_one_that_does() {
    let selection = select(&["action:build", "validation:gone"]);
    assert!(selection.resolve(&building()).is_err());
}

#[test]
fn a_kind_is_not_a_name_of_the_other_kind() {
    // `closure` is a validation; selecting it as an action matches nothing, and the other way.
    assert!(select(&["action:closure"]).resolve(&building()).is_err());
    assert!(select(&["validation:build"]).resolve(&building()).is_err());
}
