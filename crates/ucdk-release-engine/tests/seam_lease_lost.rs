// SPDX-License-Identifier: Apache-2.0
//! A build system that refuses a test because the lease is lost, without a progress event that
//! said so: the request ends as interrupted, and the action is not confirmed.

mod common;

use common::Rig;
use ucdk_release_proto::v1::{BuildSystemKind, Reason, Verdict};
use ucdk_release_seams::{
    BuildSystem, EntryWriter, Heartbeat, Invocation, Located, SeamError, TestOutcome,
};

/// A build system that answers every test with a refusal for a lost lease.
struct LeaseLost;

impl BuildSystem for LeaseLost {
    fn kind(&self) -> BuildSystemKind {
        BuildSystemKind::Buck2
    }

    fn build(&mut self, _: &Invocation<'_>, _: &dyn Heartbeat) -> Result<Vec<Located>, SeamError> {
        Err(SeamError::Permanent("builds nothing".to_owned()))
    }

    fn test(&mut self, _: &Invocation<'_>, _: &dyn Heartbeat) -> Result<TestOutcome, SeamError> {
        Err(SeamError::Refused(Reason::LeaseLost))
    }

    fn copy_out(&mut self, _: &Located, _: &mut dyn EntryWriter) -> Result<(), SeamError> {
        Err(SeamError::Permanent("copies nothing".to_owned()))
    }
}

#[test]
fn a_build_system_that_refuses_a_test_for_a_lost_lease_interrupts_the_request() {
    let mut rig = Rig::new("facts_lease_lost");
    let outcome = rig.handle_with(&mut LeaseLost, "build", &["action:test"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Interrupted, Reason::LeaseLost)
    );
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_ne!(state, ucdk_ledger::State::Confirmed);
}
