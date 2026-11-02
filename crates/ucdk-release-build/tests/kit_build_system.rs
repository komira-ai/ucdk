// SPDX-License-Identifier: Apache-2.0
//! The build-system kit run against `Buck2` and `Bazel` over a scripted process runner and the
//! fake files: each scenario is a test, run once for each kind. The scripted runner answers as
//! the tool of the kind would in the world of the scenario; the build tools themselves are not
//! started (the argument vectors and listings they are held to are confirmed against the real
//! binaries by the on-demand job of the design).

mod support;

use support::scripted::Rig;
use ucdk_release_proto::v1::BuildSystemKind;
use ucdk_release_seams::kit::{build_system_scenarios, run_build_system_kit};

fn rigs() -> [Rig; 2] {
    [
        Rig {
            kind: BuildSystemKind::Buck2,
        },
        Rig {
            kind: BuildSystemKind::Bazel,
        },
    ]
}

macro_rules! scenario_tests {
    ($($name:ident),+ $(,)?) => {
        $(
            #[test]
            fn $name() {
                for rig in rigs() {
                    support::run_scenario(&build_system_scenarios(), stringify!($name), &rig);
                }
            }
        )+

        #[test]
        fn every_scenario_of_the_kit_has_its_test() {
            let named: Vec<&str> = vec![$(stringify!($name)),+];
            let kit: Vec<&str> = build_system_scenarios::<Rig>().iter().map(|s| s.name).collect();
            assert_eq!(named, kit);
        }
    };
}

scenario_tests!(
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
fn the_whole_kit_passes_on_buck2_and_on_bazel() {
    for rig in rigs() {
        run_build_system_kit(&rig).assert_conforms();
    }
}
