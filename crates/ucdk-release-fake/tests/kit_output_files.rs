// SPDX-License-Identifier: Apache-2.0
//! The `OutputFiles` kit run against `FakeOutputFiles`: each scenario is a test of the fake.

#[macro_use]
mod rigs;

use rigs::output_files::FakeFilesRig;
use ucdk_release_seams::kit::{output_files_scenarios, run_output_files_kit};

kit_tests!(
    FakeFilesRig, [FakeFilesRig::conforming()], output_files_scenarios;
    a_listing_is_sorted_relative_paths,
    a_listing_names_the_kind_of_every_entry,
    a_listed_symlink_is_never_descended,
    stat_tells_a_file_a_directory_a_symlink_and_nothing_apart,
    a_missing_path_is_none_from_stat_and_permanent_from_list_and_read,
    listing_a_file_and_reading_a_directory_or_a_symlink_are_refused,
    a_single_file_output_is_read_without_a_rel,
    a_report_file_is_read_directly,
    reading_a_directory_is_refused_even_when_it_is_the_located_output,
    a_path_that_is_relative_or_climbs_is_refused,
    an_entry_of_a_listed_directory_is_read_through_its_joined_path,
    a_read_returns_the_file_as_it_is_at_the_call,
    no_call_changes_the_tree,
    a_dot_part_a_repeated_slash_and_a_trailing_slash_name_the_path_they_stand_in,
);

#[test]
fn the_whole_kit_passes_on_the_fake() {
    run_output_files_kit(&FakeFilesRig::conforming()).assert_conforms();
}
