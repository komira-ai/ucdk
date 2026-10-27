// SPDX-License-Identifier: Apache-2.0
//! The publisher kit run against `FakeRegistry`: each scenario is a test of the fake, on every
//! rig of the fake that can run it. A scenario that a rig lacks what it needs for is skipped on
//! that rig, and a test passes only if the scenario ran on at least one.

#[macro_use]
mod rigs;

use rigs::publisher::FakePublisherRig;
use rigs::publisher_plain::PlainRig;
use ucdk_release_seams::kit::{ChannelShape, publisher_scenarios, run_publisher_kit};

kit_tests!(
    FakePublisherRig,
    [
        FakePublisherRig::conforming(),
        FakePublisherRig::sealing(),
        FakePublisherRig::sealing().with_shapes(vec![ChannelShape::without_staging()]),
        FakePublisherRig::narrow(),
        FakePublisherRig::narrow_yanking_a_version(),
        FakePublisherRig::per_coordinate(),
        FakePublisherRig::yanking_a_version(),
        FakePublisherRig::unconditional(),
        FakePublisherRig::restricted(),
        FakePublisherRig::pairing(),
        FakePublisherRig::refusing_writer_reads(),
    ],
    publisher_scenarios;
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
    a_commit_of_staged_state_that_vanished_is_not_committed_and_a_restage_works,
    a_retry_of_a_commit_that_is_becoming_readable_is_committed_or_duplicate_and_never_a_second_write,
    a_coordinate_the_channel_cannot_hold_is_refused_before_anything_is_sent,
    a_sealed_version_refuses_a_later_platform_and_observes_it_absent,
    a_burned_coordinate_is_neither_absent_nor_present_nor_committable,
    a_registry_that_echoes_the_declared_digest_fails_readback,
    the_declarations_of_a_rig_hold_of_what_it_builds,
);

/// The kit ran in full, and a rig that skips nothing has passed every scenario.
#[test]
fn the_whole_kit_passes_on_the_fake() {
    let report = run_publisher_kit(&FakePublisherRig::sealing());
    report.assert_conforms();
    assert_eq!(report.skipped(), Vec::<&str>::new());
    assert_eq!(report.passed(), report.names());
}

/// The conforming fake does not seal a version, so it skips the scenario about one, says why, and
/// passes the other forty-five.
#[test]
fn the_fake_that_does_not_seal_a_version_skips_that_scenario_and_passes_the_rest() {
    let report = run_publisher_kit(&FakePublisherRig::conforming());
    report.assert_conforms();
    assert_eq!(
        report.skipped(),
        vec!["a_sealed_version_refuses_a_later_platform_and_observes_it_absent"]
    );
    assert_eq!(report.passed().len(), report.names().len() - 1);
}

/// A rig that takes every default of the kit's hooks has the literal coordinates and bytes of the
/// kit, holds every coordinate, seals no version and burns no number, so the three scenarios
/// about those are skipped for it, with the reason, and it passes the other forty-three.
#[test]
fn a_rig_that_takes_the_default_of_every_hook_passes_what_it_can_run() {
    let report = run_publisher_kit(&PlainRig::new());
    report.assert_conforms();
    assert_eq!(
        report.skipped(),
        vec![
            "a_coordinate_the_channel_cannot_hold_is_refused_before_anything_is_sent",
            "a_sealed_version_refuses_a_later_platform_and_observes_it_absent",
            "a_burned_coordinate_is_neither_absent_nor_present_nor_committable",
        ]
    );
    assert_eq!(report.passed().len(), report.names().len() - 3);
}

/// A registry with a grammar of its own passes the kit through the hooks of its rig.
#[test]
fn a_rig_with_a_grammar_of_its_own_passes_the_kit() {
    let report = run_publisher_kit(&FakePublisherRig::narrow());
    report.assert_conforms();
    assert_eq!(report.skipped(), Vec::<&str>::new());
}
