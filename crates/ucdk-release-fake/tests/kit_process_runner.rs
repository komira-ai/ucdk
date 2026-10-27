// SPDX-License-Identifier: Apache-2.0
//! The process-runner kit run against `FakeProcessRunner`: each scenario is a test of the fake.

#[macro_use]
mod rigs;

use rigs::process_runner::FakeProcessRig;
use ucdk_release_seams::kit::{process_runner_scenarios, run_process_runner_kit};

kit_tests!(
    FakeProcessRig, [FakeProcessRig::conforming()], process_runner_scenarios;
    a_bare_name_is_resolved_on_the_childs_path_and_never_the_runners,
    a_name_found_nowhere_on_the_childs_path_is_unstartable,
    argv_and_environment_are_exactly_what_was_given,
    no_shell_interprets_an_argument,
    exit_code_and_signal_are_distinguished,
    unstartable_is_not_a_nonzero_exit,
    output_is_bounded_and_its_tail_is_kept,
    the_runner_beats_on_progress_and_stops_the_child_when_told,
    a_stopped_child_and_its_process_group_are_gone,
);

#[test]
fn the_whole_kit_passes_on_the_fake() {
    run_process_runner_kit(&FakeProcessRig::conforming()).assert_conforms();
}
