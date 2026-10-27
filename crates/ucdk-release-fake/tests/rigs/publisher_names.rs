// SPDX-License-Identifier: Apache-2.0
//! The names of the scenarios of the publisher kit, which the tables of its self-test refer to.

pub const OBSERVE_ABSENT: &str = "observe_absent_is_not_an_error";
pub const STAGE_OBSERVE: &str = "stage_then_observe_returns_staged_with_the_same_digest";
pub const STRANGER_STAGED: &str = "a_staged_coordinate_is_not_readable_by_a_stranger";
pub const STAGE_GRANT: &str = "a_stage_grant_observes_the_staged_state_it_made";
pub const COMMIT_GRANT: &str = "a_commit_grant_observes_it_too";
pub const COMMIT_READABLE: &str =
    "commit_makes_the_staged_coordinates_readable_with_the_same_digest";
pub const STAGE_PRESENT: &str = "stage_of_present_same_is_duplicate_and_changes_nothing";
pub const COMMIT_OVER: &str = "commit_over_present_different_leaves_old_bytes";
pub const ABORT: &str = "abort_gives_staged_state_back_and_never_touches_a_committed_coordinate";
pub const SET_COMMIT: &str = "a_set_commit_makes_every_coordinate_readable_or_none";
pub const NOT_STAGE_BYTES: &str = "a_channel_that_does_not_stage_commits_with_the_bytes";
pub const NO_DIGEST: &str = "no_digest_exposed_is_refused";
pub const YANK: &str = "yank_keeps_bytes_fetchable";
pub const YANKED_COMMIT: &str = "yanked_coordinate_cannot_be_committed_again";
pub const LOST: &str = "lost_answer_is_resolvable_by_observe";
pub const LIST: &str = "list_may_lag_but_observe_is_exact";
pub const PUBLIC_READ: &str = "public_read_needs_no_grant";
pub const FOREIGN: &str = "a_grant_for_another_repository_sends_nothing";
pub const PHASE: &str = "a_grant_of_the_wrong_phase_is_denied_and_nothing_happens";
pub const EXPIRED: &str = "an_expired_grant_is_refused_and_nothing_happens";
pub const NEWEST: &str = "newest_lists_every_held_coordinate_of_the_name";
pub const STAMP: &str = "observe_returns_the_stamp_it_was_given";
pub const OTHERS_OBSERVE: &str = "a_granted_observe_sees_a_coordinate_committed_by_another_writer";
pub const GROUP_THEN_INDEX: &str = "a_private_observe_reads_the_group_then_the_index";
pub const ANOTHER_GROUP: &str =
    "a_coordinate_committed_in_another_group_reads_as_present_never_absent";
pub const FAILED_READ: &str =
    "a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent";
pub const DECLARED_SET: &str =
    "observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set";
pub const EMPTY_SET: &str =
    "observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates";
pub const READ_GRANT: &str =
    "a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit";
pub const RACE: &str = "commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit";
pub const TRANSIENT: &str = "transient_after_commit_is_observed_before_retry";
pub const DUP_OTHER: &str = "duplicate_over_other_bytes_leaves_the_old_bytes";
pub const SECOND_COMMIT: &str = "a_second_commit_of_the_same_bytes_is_duplicate";
pub const OWN_STAGED: &str = "a_stage_over_ones_own_staged_coordinate_is_duplicate";
pub const COMMIT_CARRIES: &str = "a_commit_carries_exactly_what_was_staged";
pub const DUPLICATE_SET: &str = "a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was";
pub const NAMED_TWICE: &str = "a_set_that_names_a_coordinate_twice_is_refused";
pub const WRITER_READS: &str = "a_stage_or_commit_grant_reads_no_staged_state_but_through_observe";
pub const NOT_STAGING: &str = "a_channel_that_does_not_stage_creates_a_coordinate_once";
pub const VANISHED: &str =
    "a_commit_of_staged_state_that_vanished_is_not_committed_and_a_restage_works";
pub const BECOMING: &str = "a_retry_of_a_commit_that_is_becoming_readable_is_committed_or_duplicate_and_never_a_second_write";
pub const UNREPRESENTABLE: &str =
    "a_coordinate_the_channel_cannot_hold_is_refused_before_anything_is_sent";
pub const SEALED: &str = "a_sealed_version_refuses_a_later_platform_and_observes_it_absent";
pub const BURNED: &str = "a_burned_coordinate_is_neither_absent_nor_present_nor_committable";
pub const ECHO: &str = "a_registry_that_echoes_the_declared_digest_fails_readback";
pub const DECLARES: &str = "the_declarations_of_a_rig_hold_of_what_it_builds";
