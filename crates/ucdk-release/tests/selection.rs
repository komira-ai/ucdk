// SPDX-License-Identifier: Apache-2.0
//! Selection: what a run is asked to do, and what a selector that matches nothing says.

mod common;

use common::*;
use ucdk_release::selection::{Item, Selection, SelectionError, Selector};
use ucdk_release_proto::v1::{Reason, Stage, Subject, Trigger};

fn texts(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// A stage of two steps, each with a validation.
fn two_steps() -> Stage {
    stage(
        "build",
        Trigger::Release,
        &[],
        vec![
            build_step(
                "one",
                "any",
                &["tool"],
                vec![validation("v-one", "fake", Subject::LocalSet)],
            ),
            build_step(
                "two",
                "any",
                &["tool"],
                vec![validation("v-two", "fake", Subject::LocalSet)],
            ),
        ],
    )
}

fn step(name: &str, validations: bool) -> Item {
    Item::Step {
        step: name.to_owned(),
        validations,
    }
}

#[test]
fn any_selector_is_selective() {
    assert!(!Selection::default().is_selective());
    assert!(!Selection::parse(&[]).expect("empty").is_selective());
    assert!(
        Selection::parse(&texts(&["step:one"]))
            .expect("step")
            .is_selective()
    );
    assert!(
        Selection::parse(&texts(&["validation:v-one"]))
            .expect("validation")
            .is_selective()
    );
}

#[test]
fn a_selector_is_a_kind_and_a_name() {
    for bad in [
        "",
        "one",
        "step",
        "step:",
        "validation:",
        "stage:one",
        "Step:one",
    ] {
        let err = Selection::parse(&texts(&[bad])).expect_err(bad);
        assert_eq!(err, SelectionError::Malformed(bad.to_owned()));
        assert_eq!(err.reason(), Reason::Selector);
        assert!(err.to_string().contains("step:<name>"), "{err}");
    }
}

#[test]
fn selectors_are_sorted_bytewise_and_each_counts_once() {
    let selection = Selection::parse(&texts(&[
        "validation:b",
        "step:z",
        "step:a",
        "validation:b",
    ]))
    .expect("selectors");
    assert_eq!(selection.texts(), ["step:a", "step:z", "validation:b"]);
    assert_eq!(Selector::Step("x".to_owned()).to_string(), "step:x");
    assert_eq!(
        Selector::Validation("x".to_owned()).to_string(),
        "validation:x"
    );
}

#[test]
fn no_selector_runs_every_step_with_its_validations() {
    let items = Selection::default()
        .resolve(&two_steps())
        .expect("resolved");
    assert_eq!(items, [step("one", true), step("two", true)]);
}

#[test]
fn a_step_selector_runs_the_step_without_its_validations() {
    let selection = Selection::parse(&texts(&["step:two"])).expect("selection");
    assert_eq!(
        selection.resolve(&two_steps()).expect("resolved"),
        [step("two", false)]
    );
}

#[test]
fn a_validation_selector_runs_that_validation_alone() {
    let selection = Selection::parse(&texts(&["validation:v-one"])).expect("selection");
    assert_eq!(
        selection.resolve(&two_steps()).expect("resolved"),
        [Item::Validation {
            step: "one".to_owned(),
            name: "v-one".to_owned()
        }]
    );
}

#[test]
fn selected_parts_run_in_the_order_of_the_stage() {
    let selection =
        Selection::parse(&texts(&["validation:v-two", "step:one", "step:two"])).expect("selection");
    assert_eq!(
        selection.resolve(&two_steps()).expect("resolved"),
        [
            step("one", false),
            step("two", false),
            Item::Validation {
                step: "two".to_owned(),
                name: "v-two".to_owned()
            }
        ]
    );
}

#[test]
fn no_match_lists_choices() {
    let selection = Selection::parse(&texts(&["step:three"])).expect("selection");
    let err = selection
        .resolve(&two_steps())
        .expect_err("matches nothing");
    assert_eq!(err.reason(), Reason::SelectorNoMatch);
    let SelectionError::NoMatch { selector, choices } = &err else {
        panic!("a no-match error");
    };
    assert_eq!(selector, &Selector::Step("three".to_owned()));
    assert_eq!(
        choices,
        &[
            Selector::Step("one".to_owned()),
            Selector::Step("two".to_owned()),
            Selector::Validation("v-one".to_owned()),
            Selector::Validation("v-two".to_owned()),
        ]
    );
    let text = err.to_string();
    for wanted in [
        "step:three",
        "step:one",
        "step:two",
        "validation:v-one",
        "validation:v-two",
    ] {
        assert!(text.contains(wanted), "{text}");
    }
}

#[test]
fn a_selector_that_matches_nothing_refuses_even_beside_one_that_does() {
    let selection = Selection::parse(&texts(&["step:one", "validation:gone"])).expect("selection");
    assert!(selection.resolve(&two_steps()).is_err());
}

#[test]
fn a_kind_is_not_a_name_of_the_other_kind() {
    // `v-one` is a validation; selecting it as a step matches nothing.
    let selection = Selection::parse(&texts(&["step:v-one"])).expect("selection");
    assert!(selection.resolve(&two_steps()).is_err());
}
