// SPDX-License-Identifier: Apache-2.0
//! The build-system kit run against `FakeBuildSystem`, as a Buck2 and as a Bazel: each scenario
//! is a test of the fake.

#[macro_use]
mod rigs;

use rigs::build_system::FakeBuildRig;
use ucdk_release_proto::v1::BuildSystemKind;
use ucdk_release_seams::kit::{build_system_scenarios, run_build_system_kit};

fn both_kinds() -> Vec<FakeBuildRig> {
    vec![
        FakeBuildRig::conforming(BuildSystemKind::Buck2),
        FakeBuildRig::conforming(BuildSystemKind::Bazel),
    ]
}

kit_tests!(
    FakeBuildRig, both_kinds(), build_system_scenarios;
    same_command_same_bytes,
    base_flags_come_before_target_flags_in_order,
    a_target_environment_value_replaces_the_base_value_of_the_same_key,
    nothing_is_inherited_from_the_process_environment,
    placeholders_are_substituted_in_flag_and_environment_values,
    the_stamp_reaches_a_build_only_through_placeholders,
    ucdk_owned_flags_come_after_the_authors,
    a_build_lists_exactly_one_output,
    no_output_is_build_output,
    two_outputs_are_build_output,
    a_symlink_in_an_output_is_refused,
    a_failed_build_is_permanent_with_the_tail_of_its_output,
    unstartable_is_not_failed,
    nothing_run_specific_in_the_bytes,
    copy_out_writes_only_into_the_entry_it_is_given,
    a_test_target_that_runs_nothing_is_not_a_pass,
    a_failing_test_target_is_not_a_pass,
    a_test_report_that_cannot_be_read_is_not_a_pass,
    the_build_system_beats_on_progress_and_stops_when_told,
);

#[test]
fn the_whole_kit_passes_on_the_fake() {
    for rig in both_kinds() {
        run_build_system_kit(&rig).assert_conforms();
    }
}
