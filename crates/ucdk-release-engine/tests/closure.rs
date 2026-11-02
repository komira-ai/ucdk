// SPDX-License-Identifier: Apache-2.0
//! The closure rules (`ucdk_release::closure::set_closure`) through the engine's dispatch of
//! `validation:closure`: exact requirement pins, and a members list that must equal the plain
//! members of the set. Each case seals a set, runs the validation and reads the rows it
//! reported and the state of its record.

mod common;

use std::collections::BTreeMap;

use common::{Rig, manifest};
use ucdk_ledger::State;
use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{Member, Reason, RowState, Verdict};

/// One member of a set: its name, its version, and its labels.
type Spec<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

fn member(spec: &Spec<'_>) -> Member {
    let (name, version, labels) = *spec;
    let labels: BTreeMap<String, String> = labels
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    let mut found = manifest(name, &labels);
    found.version = version.to_owned();
    assert!(matches!(found.metadata, Some(Metadata::FileMeta(_))));
    Member {
        manifest: Some(found),
    }
}

/// What the closure said of a set: the verdict, the message, the rows as (name, state, detail),
/// and the state of the record of the validation.
struct Seen {
    verdict: Verdict,
    reason: Reason,
    message: String,
    rows: Vec<(String, RowState, String)>,
    state: State,
}

fn close(case: &str, specs: &[Spec<'_>]) -> Seen {
    let mut rig = Rig::new(&format!("closure_{case}"));
    rig.seal_with(specs.iter().map(member).collect());
    let outcome = rig.handle("build", &["validation:closure"]);
    let (_, state) = rig.ledger.validation("build", "closure").expect("record");
    assert_eq!(rig.ledger.lease(), None, "{case}");
    Seen {
        verdict: outcome.verdict,
        reason: outcome.reason,
        message: outcome.message,
        rows: outcome.validations[0]
            .rows
            .iter()
            .map(|r| {
                (
                    r.name.clone(),
                    RowState::try_from(r.state).expect("a row state"),
                    r.detail.clone(),
                )
            })
            .collect(),
        state,
    }
}

fn row(name: &str) -> String {
    format!("{name}: every requirement names a member of the set")
}

const CLI: &str = "ucdk-cli";
const LIB: &str = "ucdk-lib";
const ALL: &str = "ucdk-all";

/// The set of the example plus an aggregate `ucdk-all` whose `members` label is `listed`.
fn with_list<'a>(listed: &'a [(&'a str, &'a str)]) -> [Spec<'a>; 3] {
    [
        (CLI, "1.2.3", &[]),
        (LIB, "1.2.3", &[]),
        (ALL, "1.2.3", listed),
    ]
}

fn failed_only(seen: &Seen, name: &str, detail: &str) {
    assert_eq!(seen.verdict, Verdict::ValidationFailed, "{}", seen.message);
    assert_eq!(seen.reason, Reason::Validation);
    assert_eq!(seen.state, State::Intent, "a failure leaves the intent");
    let failed: Vec<&(String, RowState, String)> = seen
        .rows
        .iter()
        .filter(|r| r.1 == RowState::RowFailed)
        .collect();
    assert_eq!(
        failed,
        [&(row(name), RowState::RowFailed, detail.to_owned())],
        "{:?}",
        seen.rows
    );
    assert_eq!(seen.message, format!("{} ({detail})", row(name)));
}

#[test]
fn a_set_whose_pins_match_and_whose_members_list_is_equal_is_confirmed() {
    let seen = close(
        "valid",
        &[
            (CLI, "1.2.3", &[("requires", "ucdk-lib==1.2.3")]),
            (LIB, "1.2.3", &[]),
            (
                ALL,
                "1.2.3",
                &[("members", "ucdk-cli==1.2.3, ucdk-lib==1.2.3")],
            ),
        ],
    );
    assert_eq!(seen.verdict, Verdict::Succeeded, "{}", seen.message);
    assert_eq!(seen.state, State::Confirmed);
    assert_eq!(
        seen.rows,
        [
            (
                row(ALL),
                RowState::RowPassed,
                "lists ucdk-cli==1.2.3, ucdk-lib==1.2.3, exactly the plain members of the set"
                    .to_owned()
            ),
            (
                row(CLI),
                RowState::RowPassed,
                "requires ucdk-lib==1.2.3, all members".to_owned()
            ),
            (
                row(LIB),
                RowState::RowPassed,
                "it declares no requirements".to_owned()
            ),
        ]
    );
}

#[test]
fn a_requirement_pinned_to_another_version_fails_its_member() {
    let seen = close(
        "wrong_requirement_pin",
        &[
            (CLI, "1.2.3", &[("requires", "ucdk-lib==9.9.9")]),
            (LIB, "1.2.3", &[]),
        ],
    );
    failed_only(
        &seen,
        CLI,
        "requires ucdk-lib==9.9.9, but ucdk-lib is at 1.2.3",
    );
    assert_eq!(seen.rows[1].1, RowState::RowPassed);
}

#[test]
fn a_members_list_pinned_to_another_version_fails_the_aggregate() {
    let seen = close(
        "wrong_list_pin",
        &with_list(&[("members", "ucdk-cli==1.2.3,ucdk-lib==1.2")]),
    );
    failed_only(&seen, ALL, "lists ucdk-lib==1.2, but ucdk-lib is at 1.2.3");
}

#[test]
fn a_members_list_that_omits_a_member_fails_the_aggregate() {
    let seen = close("omitted", &with_list(&[("members", "ucdk-cli")]));
    failed_only(
        &seen,
        ALL,
        "does not list ucdk-lib, which the set holds as a plain member",
    );
}

#[test]
fn a_members_list_with_an_extra_name_fails_the_aggregate() {
    let seen = close(
        "extra",
        &with_list(&[("members", "ucdk-cli,ucdk-lib,ucdk-ghost")]),
    );
    failed_only(
        &seen,
        ALL,
        "lists ucdk-ghost, which the set does not hold as a plain member",
    );

    // The aggregate is not a plain member of itself.
    let seen = close(
        "lists_itself",
        &with_list(&[("members", "ucdk-cli,ucdk-lib,ucdk-all")]),
    );
    failed_only(
        &seen,
        ALL,
        "lists ucdk-all, which the set does not hold as a plain member",
    );
}

#[test]
fn a_members_list_that_names_a_member_twice_fails_the_aggregate() {
    let seen = close(
        "duplicate",
        &with_list(&[("members", "ucdk-cli,ucdk-lib,ucdk-cli")]),
    );
    failed_only(&seen, ALL, "lists ucdk-cli more than once");
}

#[test]
fn two_members_lists_fail_both_aggregates() {
    let list = &[("members", "ucdk-cli,ucdk-lib")];
    let seen = close(
        "two_lists",
        &[
            (CLI, "1.2.3", &[]),
            (LIB, "1.2.3", &[]),
            (ALL, "1.2.3", list),
            ("ucdk-more", "1.2.3", list),
        ],
    );
    assert_eq!(seen.verdict, Verdict::ValidationFailed, "{}", seen.message);
    assert_eq!(seen.state, State::Intent);
    let said = "2 members carry a members list (ucdk-all, ucdk-more); at most one may";
    let states: Vec<(&str, RowState)> = seen.rows.iter().map(|r| (r.0.as_str(), r.1)).collect();
    let (cli, lib, all, more) = (row(CLI), row(LIB), row(ALL), row("ucdk-more"));
    // The rows come in the order of the sealed set, which sorts its members by name.
    assert_eq!(
        states,
        [
            (all.as_str(), RowState::RowFailed),
            (cli.as_str(), RowState::RowPassed),
            (lib.as_str(), RowState::RowPassed),
            (more.as_str(), RowState::RowFailed),
        ]
    );
    assert_eq!(seen.rows[0].2, said);
    assert_eq!(seen.rows[3].2, said);
    assert_eq!(
        seen.message,
        format!("{all} ({said}); {more} ({said})"),
        "both failures are named"
    );
}
