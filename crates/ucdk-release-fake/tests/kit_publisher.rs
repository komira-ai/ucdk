// SPDX-License-Identifier: Apache-2.0
//! The publisher kit run against `FakeRegistry`: each scenario is a test of the fake.

#[macro_use]
mod rigs;

use rigs::publisher::FakePublisherRig;
use ucdk_release_seams::kit::{publisher_scenarios, run_publisher_kit};

kit_tests!(
    FakePublisherRig, [FakePublisherRig::conforming()], publisher_scenarios;
    observe_absent_is_not_an_error,
    stage_then_observe_returns_staged_with_the_same_digest,
    a_staged_coordinate_is_not_readable_by_a_stranger,
    a_stage_grant_observes_the_staged_state_it_made,
    a_commit_grant_observes_it_too,
    commit_makes_the_staged_coordinates_readable_with_the_same_digest,
    stage_of_present_same_is_duplicate_and_changes_nothing,
    commit_over_present_different_leaves_old_bytes,
    abort_gives_staged_state_back_and_never_touches_a_committed_coordinate,
    a_set_commit_makes_every_coordinate_readable_or_none,
    a_channel_that_does_not_stage_commits_with_the_bytes,
    no_digest_exposed_is_refused,
    yank_keeps_bytes_fetchable,
    yanked_coordinate_cannot_be_committed_again,
    lost_answer_is_resolvable_by_observe,
    list_may_lag_but_observe_is_exact,
    public_read_needs_no_grant,
    a_grant_for_another_repository_sends_nothing,
    a_grant_of_the_wrong_phase_is_denied_and_nothing_happens,
    an_expired_grant_is_refused_and_nothing_happens,
    newest_lists_every_held_coordinate_of_the_name,
    observe_returns_the_stamp_it_was_given,
    a_granted_observe_sees_a_coordinate_committed_by_another_writer,
    a_private_observe_reads_the_group_then_the_index,
    a_coordinate_committed_in_another_group_reads_as_present_never_absent,
    a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent,
    observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set,
    observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates,
    a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit,
    commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit,
    transient_after_commit_is_observed_before_retry,
    duplicate_over_other_bytes_leaves_the_old_bytes,
    a_second_commit_of_the_same_bytes_is_duplicate,
    a_stage_over_ones_own_staged_coordinate_is_duplicate,
    a_commit_carries_exactly_what_was_staged,
    a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was,
    a_set_that_names_a_coordinate_twice_is_refused,
    a_stage_or_commit_grant_reads_no_staged_state_but_through_observe,
    a_channel_that_does_not_stage_creates_a_coordinate_once,
);

#[test]
fn the_whole_kit_passes_on_the_fake() {
    run_publisher_kit(&FakePublisherRig::conforming()).assert_conforms();
}
