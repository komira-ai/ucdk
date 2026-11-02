// SPDX-License-Identifier: Apache-2.0
//! Selections handed to the engine whole: an action and a validation in one request, repeated
//! selectors, the order the selectors are written in, items already confirmed beside items that
//! run, and a selector that matches nothing. Each item selected runs once, in the order of the
//! stage.

mod common;

use common::{Rig, test_row};
use ucdk_ledger::State;
use ucdk_release_proto::v1::{Reason, Verdict};

const UNIT: &str = "//release:unit";
const LINT: &str = "//release:lint";
const SET_TEST: &str = "//release:set_test";

/// The labels the build system was asked to test, in order.
fn tested(rig: &Rig) -> Vec<String> {
    rig.build
        .commands()
        .iter()
        .map(|c| c.argv.last().cloned().expect("a label"))
        .collect()
}

fn action_names(outcome: &ucdk_release_engine::Outcome) -> Vec<&str> {
    outcome.actions.iter().map(|a| a.name.as_str()).collect()
}

fn validation_names(outcome: &ucdk_release_engine::Outcome) -> Vec<&str> {
    outcome
        .validations
        .iter()
        .map(|v| v.name.as_str())
        .collect()
}

#[test]
fn an_action_and_a_validation_in_one_request_run_once_each_in_the_order_of_the_stage() {
    // Written action first, validation second; the stage has the build action's validation
    // before the `test` action.
    let mut rig = Rig::new("selection_both");
    rig.seal(None);
    let outcome = rig.handle("build", &["action:test", "validation:closure"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    assert_eq!(validation_names(&outcome), ["closure"]);
    assert_eq!(action_names(&outcome), ["test"]);
    assert_eq!(tested(&rig), [SET_TEST], "the test was asked for once");
    assert_eq!(outcome.actions[0].rows[0].name, test_row(SET_TEST));
    assert_eq!(outcome.validations[0].rows.len(), 2);
    let (_, state) = rig.ledger.validation("build", "closure").expect("record");
    assert_eq!(state, State::Confirmed);
    let (_, state) = rig.ledger.action("build", "test").expect("record");
    assert_eq!(state, State::Confirmed);
    assert_eq!(rig.ledger.lease(), None);
}

#[test]
fn selectors_in_reverse_order_and_repeated_select_each_item_once_in_stage_order() {
    let mut rig = Rig::new("selection_order");
    let outcome = rig.handle(
        "verify",
        &["action:lint", "action:unit", "action:lint", "action:unit"],
    );
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    assert_eq!(action_names(&outcome), ["unit", "lint"]);
    assert_eq!(tested(&rig), [UNIT, LINT]);
    assert_eq!(outcome.actions[0].rows[0].name, test_row(UNIT));
    assert_eq!(outcome.actions[1].rows[0].name, test_row(LINT));

    // The same selection in the other order is the same request: both are now confirmed.
    let again = rig.handle("verify", &["action:unit", "action:lint"]);
    assert_eq!(again.verdict, Verdict::Noop);
    assert_eq!(action_names(&again), ["unit", "lint"]);
    assert_eq!(tested(&rig), [UNIT, LINT], "nothing ran again");
}

#[test]
fn a_confirmed_item_before_an_item_that_runs_does_not_hide_the_pass() {
    let mut rig = Rig::new("selection_noop_first");
    let first = rig.handle("verify", &["action:unit"]);
    assert_eq!(first.verdict, Verdict::Succeeded, "{}", first.message);

    let both = rig.handle("verify", &["action:lint", "action:unit"]);
    assert_eq!(both.verdict, Verdict::Succeeded, "{}", both.message);
    assert_eq!(both.reason, Reason::Unspecified);
    assert_eq!(both.message, "");
    let verdicts: Vec<i32> = both.actions.iter().map(|a| a.verdict).collect();
    assert_eq!(
        (action_names(&both), verdicts),
        (
            vec!["unit", "lint"],
            vec![Verdict::Noop as i32, Verdict::Succeeded as i32]
        )
    );
    assert!(
        both.actions[0].rows.is_empty(),
        "a skipped action has no rows"
    );
    assert_eq!(tested(&rig), [UNIT, LINT], "unit ran once, lint once");
}

#[test]
fn a_confirmed_item_after_an_item_that_runs_does_not_hide_the_pass() {
    let mut rig = Rig::new("selection_noop_last");
    let first = rig.handle("verify", &["action:lint"]);
    assert_eq!(first.verdict, Verdict::Succeeded, "{}", first.message);

    let both = rig.handle("verify", &[]);
    assert_eq!(both.verdict, Verdict::Succeeded, "{}", both.message);
    let verdicts: Vec<i32> = both.actions.iter().map(|a| a.verdict).collect();
    assert_eq!(
        (action_names(&both), verdicts),
        (
            vec!["unit", "lint"],
            vec![Verdict::Succeeded as i32, Verdict::Noop as i32]
        )
    );
    assert_eq!(
        tested(&rig),
        [LINT, UNIT],
        "lint ran in the first request only"
    );
    for name in ["unit", "lint"] {
        let (_, state) = rig.ledger.action("verify", name).expect("record");
        assert_eq!(state, State::Confirmed, "{name}");
    }
}

#[test]
fn a_validation_confirmed_beside_an_action_that_runs_is_skipped_and_the_request_passes() {
    let mut rig = Rig::new("selection_noop_validation");
    rig.seal(None);
    let first = rig.handle("build", &["validation:closure"]);
    assert_eq!(first.verdict, Verdict::Succeeded, "{}", first.message);

    let both = rig.handle("build", &["action:test", "validation:closure"]);
    assert_eq!(both.verdict, Verdict::Succeeded, "{}", both.message);
    assert_eq!(validation_names(&both), ["closure"]);
    assert_eq!(both.validations[0].verdict, Verdict::Noop as i32);
    assert!(both.validations[0].rows.is_empty());
    assert_eq!(both.actions[0].verdict, Verdict::Succeeded as i32);
    assert_eq!(tested(&rig), [SET_TEST]);

    // Everything is confirmed now: the request as a whole is a Noop, and runs nothing.
    let third = rig.handle("build", &["action:test", "validation:closure"]);
    assert_eq!(third.verdict, Verdict::Noop);
    assert_eq!(tested(&rig), [SET_TEST]);
}

#[test]
fn a_selector_that_matches_nothing_beside_a_valid_one_runs_nothing_and_calls_no_ledger() {
    let mut rig = Rig::new("selection_unmatched");
    rig.seal(None);
    let calls = rig.ledger.calls();
    let outcome = rig.handle("build", &["action:test", "validation:ghost"]);
    assert_eq!(outcome.verdict, Verdict::Refused);
    assert_eq!(outcome.reason, Reason::SelectorNoMatch);
    assert!(
        outcome
            .message
            .starts_with("validation:ghost matches nothing in the stage; "),
        "{}",
        outcome.message
    );
    assert!(outcome.actions.is_empty() && outcome.validations.is_empty());
    assert_eq!(rig.ledger.calls(), calls, "the ledger was not called");
    assert!(rig.build.commands().is_empty(), "no test ran");
    assert_eq!(rig.ledger.lease(), None);

    // The same on a rig that never touched the ledger.
    let mut fresh = Rig::new("selection_unmatched_fresh");
    let outcome = fresh.handle("verify", &["action:unit", "action:ghost"]);
    assert_eq!(outcome.reason, Reason::SelectorNoMatch);
    assert_eq!(fresh.ledger.calls(), 0);
    assert!(fresh.ledger.records().is_empty());
}
