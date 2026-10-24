// SPDX-License-Identifier: Apache-2.0
//! The contract kit run against the fake. Each scenario runs on the standard fake (kinds
//! `service` and `job`) and on the limited one, where `job` is declared unsupported: its
//! scenarios are skipped by that declaration and the rest run.

use ucdk_provider::Kind;
use ucdk_provider::kit::{self, Failure, Harness};
use ucdk_provider_fake::FakeHarness;

fn on_fake_and_fake_limited(scenario: fn(&FakeHarness) -> Result<(), Failure>) {
    scenario(&FakeHarness::standard()).expect("the standard fake conforms");
    scenario(&FakeHarness::limited()).expect("the limited fake conforms");
}

#[test]
fn kit_create_stamps_in_create_call() {
    on_fake_and_fake_limited(kit::create_stamps_in_create_call);
}

#[test]
fn kit_create_existing_is_already_exists() {
    on_fake_and_fake_limited(kit::create_existing_is_already_exists);
}

#[test]
fn kit_update_missing_is_not_found() {
    on_fake_and_fake_limited(kit::update_missing_is_not_found);
}

#[test]
fn kit_foreign_same_name_untouched() {
    on_fake_and_fake_limited(kit::foreign_same_name_untouched);
}

#[test]
fn kit_delete_absent_is_idempotent() {
    on_fake_and_fake_limited(kit::delete_absent_is_idempotent);
}

#[test]
fn kit_read_is_live_not_cached() {
    on_fake_and_fake_limited(kit::read_is_live_not_cached);
}

#[test]
fn kit_rename_invariance() {
    on_fake_and_fake_limited(kit::rename_invariance);
}

#[test]
fn kit_update_changes_digest_and_keeps_stamp() {
    on_fake_and_fake_limited(kit::update_changes_digest_and_keeps_stamp);
}

#[test]
fn kit_declared_unsupported_kinds_refuse() {
    on_fake_and_fake_limited(kit::declared_unsupported_kinds_refuse);
}

#[test]
fn kit_runs_every_scenario_on_both_fakes() {
    assert!(kit::run_all(&FakeHarness::standard()).is_empty());
    assert!(kit::run_all(&FakeHarness::limited()).is_empty());
}

#[test]
fn the_limited_fake_skips_job_by_declaration() {
    let standard = FakeHarness::standard();
    assert_eq!(standard.kinds(), [Kind::service(), Kind::job()]);
    assert!(standard.unsupported().is_empty());
    let limited = FakeHarness::limited();
    assert_eq!(limited.kinds(), [Kind::service()]);
    assert_eq!(limited.unsupported(), [Kind::job()]);
}
