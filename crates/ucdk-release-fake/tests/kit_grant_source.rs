// SPDX-License-Identifier: Apache-2.0
//! The grant-source kit run against `FakeGrantSource`: each scenario is a test of the fake.

#[macro_use]
mod rigs;

use rigs::grant_source::FakeGrantRig;
use ucdk_release_seams::kit::{grant_source_scenarios, run_grant_source_kit};

kit_tests!(
    FakeGrantRig, [FakeGrantRig::conforming()], grant_source_scenarios;
    an_anonymous_identity_needs_no_exchange,
    a_grant_covers_exactly_the_coordinates_asked,
    a_differing_claim_is_refused_before_any_exchange,
    an_exceptional_run_presents_the_exceptional_subject,
    a_used_identity_token_is_refused_the_second_time,
    an_exchange_after_expiry_gives_a_new_grant,
    a_missing_scope_is_denied,
    a_read_grant_carries_the_read_scope_and_covers_exactly_the_coordinates_asked,
    an_unreachable_issuer_is_unreachable_and_spends_nothing,
    an_expiry_is_reported_as_the_issuer_stated_it,
    more_coordinates_than_the_limit_are_refused_before_the_exchange,
    a_grant_is_never_printed_cloned_or_logged,
);

#[test]
fn the_whole_kit_passes_on_the_fake() {
    run_grant_source_kit(&FakeGrantRig::conforming()).assert_conforms();
}
