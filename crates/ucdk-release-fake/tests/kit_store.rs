// SPDX-License-Identifier: Apache-2.0
//! The store kit run against `FakeStore`: each scenario is a test of the fake.

#[macro_use]
mod rigs;

use rigs::store::FakeStoreRig;
use ucdk_release_seams::kit::{run_store_kit, store_scenarios};

kit_tests!(
    FakeStoreRig, [FakeStoreRig::conforming()], store_scenarios;
    an_entry_reads_back_what_was_written,
    an_entry_is_listed_exactly,
    a_second_begin_on_one_key_is_refused,
    keys_of_two_scopes_are_listed_apart,
    paths_that_escape_an_entry_are_refused,
    two_entries_do_not_touch_each_other,
    discard_removes_every_path_of_an_entry,
    a_missing_entry_lists_as_empty,
);

#[test]
fn the_whole_kit_passes_on_the_fake() {
    run_store_kit(&FakeStoreRig::conforming()).assert_conforms();
}
