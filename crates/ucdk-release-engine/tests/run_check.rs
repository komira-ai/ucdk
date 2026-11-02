// SPDX-License-Identifier: Apache-2.0
//! The facts of a run are checked before the lease is taken: an attempt from 1, full commit ids
//! for the revision and the stamp's source commit, and a positive build number and timestamp.
//! A run that breaks one is refused with `SOURCE`, and the ledger and the build system are never
//! called.

mod common;

use common::Rig;
use ucdk_release_proto::v1::{Reason, Verdict};

/// Runs `action:test` of `build` with `change` made to the run, and asserts it was refused
/// with `message` before any call.
fn refused(name: &str, change: impl FnOnce(&mut Rig), message: &str) {
    let mut rig = Rig::new(name);
    change(&mut rig);
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Refused, Reason::Source),
        "{name}: {}",
        outcome.message
    );
    assert_eq!(outcome.message, message, "{name}");
    assert_eq!(rig.ledger.calls(), 0, "{name}: the ledger was not called");
    assert!(rig.build.commands().is_empty(), "{name}: nothing was run");
}

#[test]
fn an_attempt_of_zero_is_refused_before_any_call() {
    refused(
        "run_attempt",
        |r| r.attempt = 0,
        "the attempt is 0; attempts count from 1",
    );
}

#[test]
fn a_revision_that_is_not_a_full_commit_id_is_refused_before_any_call() {
    refused(
        "run_revision",
        |r| r.revision = "0123456".to_owned(),
        "the run's revision \"0123456\" is not a full commit id (exactly 40 lowercase hex \
         digits; an abbreviated id is refused)",
    );
}

#[test]
fn an_abbreviated_source_commit_is_refused_before_any_call() {
    refused(
        "run_commit",
        |r| r.stamp.source_commit = "abc123".to_owned(),
        "the run's stamp: source_commit \"abc123\" is not a full commit id (exactly 40 \
         lowercase hex digits; an abbreviated id is refused)",
    );
}

#[test]
fn a_build_number_or_timestamp_of_zero_is_refused_before_any_call() {
    refused(
        "run_build",
        |r| r.stamp.build_number = 0,
        "the run's stamp: build_number 0 is not positive",
    );
    refused(
        "run_time",
        |r| r.stamp.timestamp_ms = 0,
        "the run's stamp: timestamp_ms 0 is not positive",
    );
}

#[test]
fn the_largest_build_number_and_timestamp_are_accepted() {
    let mut rig = Rig::new("run_largest");
    rig.stamp.build_number = u64::MAX;
    rig.stamp.timestamp_ms = u64::MAX;
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
}
