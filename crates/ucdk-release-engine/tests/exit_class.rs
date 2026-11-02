// SPDX-License-Identifier: Apache-2.0
//! The exit class of each way a request ends, through `ucdk_api::exit_class`. Test actions and
//! validations land nothing in a channel, so every outcome is classed with `landed` false.

mod common;

use common::{Rig, TTL, Watched};
use ucdk_api::exit_class;
use ucdk_api::v1::ExitClass;
use ucdk_ledger::{Ledger, MAX_CLOCK_SKEW};
use ucdk_release_engine::Outcome;
use ucdk_release_fake::FakeBuildSystem;
use ucdk_release_proto::v1::BuildSystemKind;

fn class(outcome: &Outcome) -> ExitClass {
    exit_class(outcome.verdict, outcome.reason, false)
}

#[test]
fn a_pass_and_a_skip_are_done() {
    let mut rig = Rig::new("class_done");
    assert_eq!(
        class(&rig.handle("build", &["action:test"])),
        ExitClass::Done
    );
    assert_eq!(
        class(&rig.handle("build", &["action:test"])),
        ExitClass::Done
    );
}

#[test]
fn a_failing_target_is_class_seven() {
    let mut rig = Rig::new("class_failed_target");
    rig.build.test_fails("//release:set_test");
    let class = class(&rig.handle("build", &["action:test"]));
    assert_eq!((class, class.code()), (ExitClass::ValidationFailed, 7));
}

#[test]
fn an_unstartable_build_system_is_class_five() {
    let mut rig = Rig::new("class_unstartable");
    rig.build.unstartable();
    let class = class(&rig.handle("build", &["action:test"]));
    assert_eq!((class, class.code()), (ExitClass::Indeterminate, 5));
}

#[test]
fn a_request_refused_before_the_ledger_or_by_a_held_lease_is_class_three() {
    let mut rig = Rig::new("class_refused");
    for outcome in [
        rig.handle("nowhere", &[]),
        rig.handle("build", &["action:nope"]),
    ] {
        assert_eq!(class(&outcome), ExitClass::Refused, "{}", outcome.message);
    }
    rig.ledger.inner.acquire("someone-else", TTL).expect("held");
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(class(&outcome), ExitClass::Refused, "{}", outcome.message);
    assert!(class(&outcome).promises_no_effect());
}

#[test]
fn a_ledger_that_refuses_its_writes_is_class_four() {
    let mut rig = Rig::new("class_ledger");
    rig.ledger.fail_writes(true);
    let class = class(&rig.handle("build", &["action:test"]));
    assert_eq!((class, class.code()), (ExitClass::FailedNoEffect, 4));
}

#[test]
fn a_lost_or_unconfirmed_lease_is_class_six() {
    let mut rig = Rig::new("class_unconfirmed");
    rig.ledger.lose_acquire_answer(true);
    let class_of_unconfirmed = class(&rig.handle("build", &["action:test"]));
    assert_eq!(class_of_unconfirmed, ExitClass::FailedChanged);

    let mut rig = Rig::new("class_lost");
    let (ledger, clock) = (rig.ledger.clone(), rig.clock.clone());
    let mut build = FakeBuildSystem::new(BuildSystemKind::Buck2, &rig.clock);
    let mut watched = Watched::new(&mut build, || {
        clock.advance(TTL + MAX_CLOCK_SKEW);
        ledger.inner.acquire("someone-else", TTL).expect("taken");
    });
    let outcome = rig.handle_with(&mut watched, "build", &["action:test"]);
    let class = class(&outcome);
    assert_eq!((class, class.code()), (ExitClass::FailedChanged, 6));
    assert!(!class.promises_no_effect());
}
