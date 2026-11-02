// SPDX-License-Identifier: Apache-2.0
//! The run record pins the run: a confirmed Test action is taken as this run's, and a validation
//! is run, only when the run record pins the run the engine was given. The action input digest
//! leaves out the run id, the timestamp and the parts of the machine the action does not read,
//! so the pin is what tells another run apart. Nothing is run or written when it differs.

mod common;

use std::collections::BTreeMap;

use common::Rig;
use ucdk_core::RecordPath;
use ucdk_ledger::{Ledger, Record as Row, Token};
use ucdk_release::machine::Checked;
use ucdk_release_proto::v1::{Reason, Verdict};

fn untouched(rig: &Rig) -> (Token, BTreeMap<RecordPath, Row>) {
    let snapshot = rig.ledger.inner.read().expect("read");
    (snapshot.token, snapshot.records)
}

/// A rig whose `action:test` of `build` is confirmed, then `change` done to it.
fn confirmed_then(name: &str, change: impl FnOnce(&mut Rig)) -> Rig {
    let mut rig = Rig::new(name);
    let first = rig.handle("build", &["action:test"]);
    assert_eq!(first.verdict, Verdict::Succeeded, "{}", first.message);
    change(&mut rig);
    rig
}

/// Asserts the request is refused for a pin that names `field`, and runs and writes nothing.
fn refused_for(rig: &mut Rig, only: &[&str], field: &str) {
    let before = untouched(rig);
    let commands = rig.build.commands().len();
    let outcome = rig.handle("build", only);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Refused, Reason::PinMismatch),
        "{}",
        outcome.message
    );
    assert_eq!(
        outcome.message,
        format!(
            "the run record pins other values for: {field}; use the machine, revision and stamp \
             the run began with"
        )
    );
    assert!(outcome.actions.is_empty() && outcome.validations.is_empty());
    assert_eq!(rig.build.commands().len(), commands, "nothing was run");
    assert_eq!(untouched(rig), before, "nothing was written");
}

#[test]
fn a_confirmed_action_is_not_taken_for_a_run_with_another_timestamp() {
    let mut rig = confirmed_then("pin_timestamp", |r| r.stamp.timestamp_ms += 1);
    refused_for(&mut rig, &["action:test"], "timestamp");
}

#[test]
fn a_confirmed_action_is_not_taken_for_a_run_with_another_id() {
    let mut rig = confirmed_then("pin_run_id", |r| r.run_id = "r2".to_owned());
    refused_for(&mut rig, &["action:test"], "run id");
}

#[test]
fn a_confirmed_action_is_not_taken_for_a_machine_changed_where_it_does_not_read() {
    let mut rig = confirmed_then("pin_machine", |r| {
        // Another stage's targets: not an input of build/test, and part of the machine.
        let mut machine = common::machine();
        common::set_targets(&mut machine, "verify", "unit", &["//release:other"]);
        r.machine = Checked::read(machine).expect("the machine passes every rule");
    });
    refused_for(&mut rig, &["action:test"], "machine");
}

#[test]
fn a_validation_is_not_run_for_another_run_than_the_one_pinned() {
    let mut rig = Rig::new("pin_validation");
    rig.seal(None);
    let first = rig.handle("build", &["action:test"]);
    assert_eq!(first.verdict, Verdict::Succeeded, "{}", first.message);
    rig.stamp.timestamp_ms += 1;
    let observes = rig.registry.observes;
    refused_for(&mut rig, &["validation:closure"], "timestamp");
    assert_eq!(rig.registry.observes, observes, "the channel was not read");
    assert!(rig.ledger.validation("build", "closure").is_none());
}

#[test]
fn a_validation_with_no_run_record_runs_and_pins_nothing() {
    let mut rig = Rig::new("pin_none");
    rig.seal(None);
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    assert!(
        rig.ledger.run_record().is_none(),
        "a validation pins nothing"
    );
}

#[test]
fn a_confirmed_action_with_inputs_that_changed_is_still_inputs_changed() {
    // A flag of the build system is an input of the action, so the digest differs: that is named first.
    let mut rig = confirmed_then("pin_inputs", |r| {
        let mut machine = common::machine();
        machine
            .build_system
            .as_mut()
            .expect("a build system")
            .flags
            .push("--another-flag".to_owned());
        r.machine = Checked::read(machine).expect("the machine passes every rule");
    });
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Refused, Reason::InputsChanged),
        "{}",
        outcome.message
    );
}
