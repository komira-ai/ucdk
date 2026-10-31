// SPDX-License-Identifier: Apache-2.0
//! Validations: `SET_CLOSURE` over a set the test writes into the fake store, and
//! `CHANNEL_READBACK` over a channel read through a publisher. Rows are derived from the subject,
//! and a validation passes only when at least one row was asserted and every row passed.

mod common;

use common::{Fetch, PLATFORM, Rig, coordinate, readback_row, validation};
use ucdk_ledger::{Ledger, State};
use ucdk_release::codec;
use ucdk_release::store::{SetScope, StoreKey};
use ucdk_release_engine::SET_FILE;
use ucdk_release_fake::coordinate_key;
use ucdk_release_proto::v1::{Reason, RowState, ValidationKind, Verdict, Visibility};
use ucdk_release_seams::{ArtifactStore as _, Grant, Publisher as _, SeamError};

/// What a case does to the rig before the read-back.
type Arrange = fn(&mut Rig);

fn closure_row(name: &str) -> String {
    format!("{name}: every requirement names a member of the set")
}

fn states(report: &ucdk_release_proto::v1::ValidationReport) -> Vec<RowState> {
    report
        .rows
        .iter()
        .map(|r| RowState::try_from(r.state).expect("a row state"))
        .collect()
}

#[test]
fn a_set_closure_validation_confirmed_for_the_same_set_is_noop() {
    let mut rig = Rig::new("closure_noop");
    rig.decoy_validation("build", "closure");
    let hash = rig.seal(None);
    let first = rig.handle("build", &["validation:closure"]);
    assert_eq!(first.verdict, Verdict::Succeeded, "{}", first.message);
    let before = rig.ledger.inner.read().expect("read");

    let second = rig.handle("build", &["validation:closure"]);
    assert_eq!(second.verdict, Verdict::Noop);
    assert_eq!(second.validations[0].verdict, Verdict::Noop as i32);
    assert!(second.validations[0].rows.is_empty());
    let after = rig.ledger.inner.read().expect("read");
    assert_eq!(after.token, before.token, "no record was written");
    assert_eq!(after.records, before.records);

    // Another set under the same name is not the set it passed: it is read again.
    let other = rig.seal_members(None, &["ucdk-cli"]);
    assert_ne!(other, hash);
    let third = rig.handle("build", &["validation:closure"]);
    assert_eq!(third.verdict, Verdict::Succeeded, "{}", third.message);
    assert_eq!(third.validations[0].rows.len(), 1);
    let (record, state) = rig.ledger.validation("build", "closure").expect("record");
    assert_eq!((state, record.set), (State::Confirmed, Some(other)));
}

#[test]
fn a_set_closure_validation_records_the_set_it_read() {
    let mut rig = Rig::new("closure_records");
    let hash = rig.seal(None);
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    let (record, state) = rig.ledger.validation("build", "closure").expect("record");
    assert_eq!(state, State::Confirmed);
    assert_eq!(record.set, Some(hash));
    assert_eq!(rig.ledger.lease(), None);
}

#[test]
fn a_readback_validation_is_never_skipped() {
    let mut rig = Rig::new("readback_never_skipped");
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    for _ in 0..2 {
        let outcome = rig.handle("publish", &["validation:visible"]);
        assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
        assert_eq!(outcome.validations[0].verdict, Verdict::Succeeded as i32);
    }
    assert_eq!(
        rig.registry.observes, 4,
        "both coordinates were read, both times"
    );
    let (record, state) = rig.ledger.validation("publish", "visible").expect("record");
    assert_eq!(state, State::Confirmed);
    assert_eq!(record.set, None, "a read-back names no set");
}

#[test]
fn a_failed_validation_leaves_its_record_unconfirmed() {
    // A closure that names a member the set does not have.
    let mut rig = Rig::new("closure_fails");
    rig.seal(Some("ghost"));
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(outcome.verdict, Verdict::ValidationFailed);
    assert_eq!(outcome.reason, Reason::Validation);
    assert!(outcome.message.contains("ghost"), "{}", outcome.message);
    let report = &outcome.validations[0];
    assert_eq!(states(report), [RowState::RowFailed, RowState::RowPassed]);
    let (_, state) = rig.ledger.validation("build", "closure").expect("record");
    assert_eq!(state, State::Intent, "a failure is not a state");
    // It is read again, not skipped.
    let again = rig.handle("build", &["validation:closure"]);
    assert_eq!(again.verdict, Verdict::ValidationFailed);

    // A read-back of coordinates nobody published.
    let mut rig = Rig::new("readback_fails");
    rig.seal(None);
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(outcome.verdict, Verdict::ValidationFailed);
    assert_eq!(outcome.reason, Reason::Validation);
    let (_, state) = rig.ledger.validation("publish", "visible").expect("record");
    assert_eq!(state, State::Intent);
}

#[test]
fn vacuous_leg_is_not_a_pass() {
    // A set with no member asserts nothing: that is a fault, and never a pass.
    let mut rig = Rig::new("vacuous");
    rig.seal_members(None, &[]);
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(outcome.verdict, Verdict::Refused);
    assert_eq!(outcome.reason, Reason::RowAccounting);
    assert!(
        outcome.message.contains("no row was derived"),
        "{}",
        outcome.message
    );
    let (_, state) = rig.ledger.validation("build", "closure").expect("record");
    assert_eq!(state, State::Intent);
}

#[test]
fn rows_are_derived_from_the_subject() {
    let mut rig = Rig::new("rows");
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    let closure = rig.handle("build", &["validation:closure"]);
    let names: Vec<&str> = closure.validations[0]
        .rows
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, [closure_row("ucdk-cli"), closure_row("ucdk-lib")]);

    let readback = rig.handle("publish", &["validation:visible"]);
    let names: Vec<&str> = readback.validations[0]
        .rows
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, [readback_row("ucdk-cli"), readback_row("ucdk-lib")]);
    assert!(
        readback_row("ucdk-cli").starts_with("registry/ucdk-cli 1.2.3 linux-x86_64 (FILE): "),
        "the channel, the coordinate and the type are in the name: {}",
        readback_row("ucdk-cli")
    );
}

#[test]
fn a_readback_validation_reads_the_channel_through_the_publisher() {
    let mut rig = Rig::new("readback_calls");
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    let reads: Vec<String> = rig
        .registry
        .inner
        .call_log()
        .iter()
        .skip_while(|c| !c.starts_with("observe"))
        .cloned()
        .collect();
    let (cli, lib) = (
        coordinate_key(&coordinate("ucdk-cli")),
        coordinate_key(&coordinate("ucdk-lib")),
    );
    assert_eq!(
        reads,
        [
            format!("observe {cli}"),
            format!("fetch {cli}"),
            format!("observe {lib}"),
            format!("fetch {lib}"),
        ],
        "each coordinate is observed, and its bytes are fetched and hashed here"
    );
    assert_eq!(rig.pause.total(), 0, "nothing was waited for");
    for row in &outcome.validations[0].rows {
        assert!(row.detail.starts_with("serves "), "{}", row.detail);
    }
}

#[test]
fn a_readback_polls_for_a_coordinate_that_is_not_served_yet() {
    // Not served at the first read, served at the second.
    let mut rig = Rig::new("readback_lag");
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    rig.registry.inner.read_lag(1);
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    assert_eq!(
        rig.pause.ticks(),
        [1],
        "one pause tick between the two reads"
    );

    // Never served: the reads and the pauses are bounded by `wait_polls` (2 in the example).
    let mut rig = Rig::new("readback_absent");
    rig.seal(None);
    rig.publish(&["ucdk-cli"]);
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(outcome.verdict, Verdict::ValidationFailed);
    let report = &outcome.validations[0];
    assert_eq!(states(report), [RowState::RowPassed, RowState::RowFailed]);
    assert!(
        report.rows[1].detail.contains("does not serve"),
        "{}",
        report.rows[1].detail
    );
    assert_eq!(rig.pause.ticks(), [1, 1]);
}

#[test]
fn a_readback_validation_is_anonymous_on_a_public_repository() {
    let mut rig = Rig::new("readback_anonymous");
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    let calls_before = rig.registry.anonymous.len();
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    let reads = &rig.registry.anonymous[calls_before..];
    assert_eq!(reads.len(), 4);
    assert!(
        reads.iter().all(|anonymous| *anonymous),
        "every read was a stranger's"
    );

    // A private repository needs a read grant, which this engine does not exchange: the read
    // is not made, and the validation says why.
    let mut rig = Rig::with("readback_private", |m| {
        m.channels[0].repositories[0].visibility = Visibility::Private as i32;
    });
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(outcome.verdict, Verdict::Indeterminate);
    assert_eq!(outcome.reason, Reason::Channel);
    let report = &outcome.validations[0];
    assert_eq!(
        states(report),
        [RowState::RowUnreadable, RowState::RowUnreadable]
    );
    assert!(
        report.rows[0].detail.contains("read grant"),
        "{}",
        report.rows[0].detail
    );
    assert_eq!(rig.registry.observes, 0, "no read was made");
}

#[test]
fn a_stage_level_readback_covers_every_publish_action() {
    let mut rig = Rig::with("readback_stage", |m| {
        common::stage_mut(m, "publish").validations.push(validation(
            "everything",
            ValidationKind::ChannelReadback,
            0,
        ));
    });
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    let outcome = rig.handle("publish", &["validation:everything"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    let names: Vec<&str> = outcome.validations[0]
        .rows
        .iter()
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(names, [readback_row("ucdk-cli"), readback_row("ucdk-lib")]);
    let (_, state) = rig
        .ledger
        .validation("publish", "everything")
        .expect("record");
    assert_eq!(state, State::Confirmed);
}

#[test]
fn a_readback_says_what_the_channel_holds_instead_of_the_set() {
    let cases: [(&str, Arrange); 5] = [
        ("burned", |rig| {
            rig.registry.inner.burn(&coordinate("ucdk-lib"));
        }),
        ("yanked", |rig| {
            let lib = coordinate("ucdk-lib");
            rig.registry
                .inner
                .yank(&Grant::none(), &lib, "withdrawn")
                .expect("yank");
        }),
        ("holds digest", |rig| {
            rig.registry
                .inner
                .present_different(&coordinate("ucdk-lib"), b"other bytes");
        }),
        ("hash to", |rig| {
            rig.registry.fetch = Fetch::Answer(Ok(Some(b"not the member".to_vec())));
        }),
        ("serves no bytes", |rig| {
            rig.registry.fetch = Fetch::Answer(Ok(None));
        }),
    ];
    for (said, arrange) in cases {
        let mut rig = Rig::new(&format!("readback_{}", said.replace(' ', "_")));
        rig.seal(None);
        rig.publish(&["ucdk-cli", "ucdk-lib"]);
        arrange(&mut rig);
        let outcome = rig.handle("publish", &["validation:visible"]);
        assert_eq!(
            outcome.verdict,
            Verdict::ValidationFailed,
            "{said}: {}",
            outcome.message
        );
        assert!(
            outcome.message.contains(said),
            "{said}: {}",
            outcome.message
        );
        let (_, state) = rig.ledger.validation("publish", "visible").expect("record");
        assert_eq!(state, State::Intent, "{said}");
    }
}

#[test]
fn a_read_that_fails_is_an_unreadable_row_and_not_a_failed_one() {
    // The fetch fails at every poll.
    let mut rig = Rig::new("fetch_fails");
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    rig.registry.fetch =
        Fetch::Answer(Err(SeamError::Unknown("the connection dropped".to_owned())));
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(outcome.verdict, Verdict::Indeterminate);
    assert_eq!(outcome.reason, Reason::Channel);
    let report = &outcome.validations[0];
    assert_eq!(
        states(report),
        [RowState::RowUnreadable, RowState::RowUnreadable]
    );
    assert!(
        report.rows[0].detail.contains("connection dropped"),
        "{}",
        report.rows[0].detail
    );
    assert_eq!(
        rig.registry.fetches, 6,
        "each coordinate was fetched at every poll"
    );

    // The observe fails.
    let mut rig = Rig::new("observe_fails");
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    rig.registry.inner.unreadable_after(0);
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(outcome.verdict, Verdict::Indeterminate);
    assert_eq!(rig.registry.fetches, 0);

    // No publisher was given for the channel.
    let mut rig = Rig::new("no_publisher");
    rig.seal(None);
    rig.with_publisher = false;
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(outcome.verdict, Verdict::Indeterminate);
    assert!(
        outcome
            .message
            .contains("no publisher was given for the channel registry"),
        "{}",
        outcome.message
    );
}

#[test]
fn a_set_that_lacks_a_package_the_channel_should_serve_is_refused() {
    let mut rig = Rig::new("lacks_member");
    rig.seal_members(None, &["ucdk-cli"]);
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(outcome.verdict, Verdict::Refused);
    assert_eq!(outcome.reason, Reason::SetMember);
    assert!(outcome.message.contains("ucdk-lib"), "{}", outcome.message);
    assert!(
        rig.ledger.validation("publish", "visible").is_none(),
        "nothing was written"
    );
    assert_eq!(rig.registry.observes, 0);
}

#[test]
fn a_set_that_cannot_be_read_is_refused_by_what_is_wrong_with_it() {
    // No record of the stage that holds the set.
    let mut rig = Rig::new("no_stage_record");
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(outcome.verdict, Verdict::Refused);
    assert_eq!(outcome.reason, Reason::StageNotAdmitted);

    // A record that names no set, or no platform.
    for (platform, set) in [(Some(PLATFORM), false), (None, true)] {
        let mut rig = Rig::new("stage_record_incomplete");
        let hash = rig.seal(None);
        rig.ledger.seed(
            &codec::Record::Stage(codec::Stage {
                stage: "build".to_owned(),
                attempt: 1,
                platform: platform.map(str::to_owned),
                set: set.then_some(hash),
                generation: Some("r1-a1".to_owned()),
                confirmed: false,
            }),
            State::Intent,
        );
        let outcome = rig.handle("build", &["validation:closure"]);
        assert_eq!(outcome.verdict, Verdict::Refused);
        assert_eq!(outcome.reason, Reason::SetHash);
    }

    // The document is not where the record says.
    let mut rig = Rig::new("no_document");
    rig.seal(None);
    rig.ledger.seed(
        &codec::Record::Stage(codec::Stage {
            stage: "build".to_owned(),
            attempt: 1,
            platform: Some(PLATFORM.to_owned()),
            set: Some(ucdk_core::Hash256::from_hex(&"a".repeat(64)).expect("a hash")),
            generation: Some("r1-a2".to_owned()),
            confirmed: false,
        }),
        State::Intent,
    );
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(outcome.reason, Reason::SetHash);
    assert!(
        outcome.message.contains("no set document"),
        "{}",
        outcome.message
    );

    // The store refuses, or cannot say.
    let mut rig = Rig::new("store_refuses");
    rig.seal(None);
    rig.store.get = Some(SeamError::Refused(Reason::SetMember));
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Refused, Reason::SetMember)
    );
    rig.store.get = Some(SeamError::Unknown("the disk is gone".to_owned()));
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Indeterminate, Reason::SetHash)
    );
    assert!(
        outcome.message.contains("the disk is gone"),
        "{}",
        outcome.message
    );

    // A document that is not a set, and a set that is not the one the record names.
    let mut rig = Rig::new("document_garbage");
    rig.seal(None);
    let key = StoreKey::set(&SetScope {
        stage: "build".to_owned(),
        platform: PLATFORM.to_owned(),
        revision: common::REVISION.to_owned(),
        generation: "r1-a1".to_owned(),
    })
    .expect("a key");
    rig.store.inner.discard(&key).expect("discard");
    rig.store
        .inner
        .begin(&key)
        .expect("begin")
        .put_file(SET_FILE, b"\xff\xff\xff")
        .expect("put");
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Refused, Reason::SetHash)
    );

    let mut rig = Rig::new("document_differs");
    rig.seal(None);
    rig.ledger.seed(
        &codec::Record::Stage(codec::Stage {
            stage: "build".to_owned(),
            attempt: 1,
            platform: Some(PLATFORM.to_owned()),
            set: Some(ucdk_core::Hash256::from_hex(&"b".repeat(64)).expect("a hash")),
            generation: Some("r1-a1".to_owned()),
            confirmed: false,
        }),
        State::Intent,
    );
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Refused, Reason::SetHash)
    );
    assert!(outcome.message.contains("names"), "{}", outcome.message);
}

#[test]
fn a_validation_that_cannot_read_or_write_the_ledger_is_a_ledger_failure() {
    // The ledger dies at the read of its records, after the lease was taken.
    let mut rig = Rig::new("validation_reads");
    rig.seal(None);
    rig.ledger.crash_at(3);
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Failed, Reason::Ledger)
    );
    rig.ledger.revive();

    // The intent cannot be written: nothing is evaluated.
    rig.ledger.fail_writes(true);
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Failed, Reason::Ledger)
    );
    assert!(rig.ledger.validation("build", "closure").is_none());
}

#[test]
fn a_validation_whose_lease_was_taken_over_while_it_read_is_not_confirmed() {
    let mut rig = Rig::new("validation_taken_over");
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    let (ledger, clock) = (rig.ledger.clone(), rig.clock.clone());
    rig.registry.on_observe = Some(Box::new(move || {
        clock.advance(common::TTL + ucdk_ledger::MAX_CLOCK_SKEW);
        ledger
            .inner
            .acquire("someone-else", common::TTL)
            .expect("taken");
    }));
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Interrupted, Reason::LeaseLost)
    );
    let (_, state) = rig.ledger.validation("publish", "visible").expect("record");
    assert_eq!(
        state,
        State::Intent,
        "the read passed, and nobody may record it"
    );
}

#[test]
fn a_validation_whose_confirmation_cannot_be_written_is_a_ledger_failure() {
    let mut rig = Rig::new("validation_confirm");
    rig.seal(None);
    rig.publish(&["ucdk-cli", "ucdk-lib"]);
    let ledger = rig.ledger.clone();
    rig.registry.on_observe = Some(Box::new(move || ledger.fail_writes(true)));
    let outcome = rig.handle("publish", &["validation:visible"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Failed, Reason::Ledger)
    );
    let (_, state) = rig.ledger.validation("publish", "visible").expect("record");
    assert_eq!(state, State::Intent);
}
