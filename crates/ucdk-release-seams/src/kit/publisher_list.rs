// SPDX-License-Identifier: Apache-2.0
//! The scenarios of the publisher kit in the order of the design, each with the gate that skips
//! it and the shapes it runs on. A scenario about what a channel does runs on every declared shape
//! that can do it, and its gate names the capability it needs (`publisher_gates`), never the
//! channel being plain or public.

use super::Scenario;
use super::publisher as base;
use super::publisher_access as access;
use super::publisher_gates::{
    needs_a_channel, needs_a_conditional_commit, needs_a_grouped_channel, needs_a_private_channel,
    needs_a_public_channel, needs_a_registry_that_can_burn, needs_a_sealing_registry,
    needs_a_set_commit, needs_a_staging_channel, needs_an_unrepresentable_coordinate,
    needs_an_unstaged_channel,
};
use super::publisher_more as more;
use super::publisher_private as private;
use super::publisher_rig::{
    PublisherRig, conditional_shapes, grouped_shapes, on_each_of, on_each_shape,
    on_each_staging_shape, on_each_unstaged_shape, private_shapes, public_shapes,
    set_commit_shapes,
};
use super::publisher_shapes as shapes;

pub fn publisher_scenarios<R: PublisherRig>() -> Vec<Scenario<R>> {
    vec![
        Scenario::new("observe_absent_is_not_an_error", |rig| {
            on_each_shape(rig, base::observe_absent_is_not_an_error::<R>);
        })
        .gated(needs_a_channel::<R>),
        Scenario::new(
            "stage_then_observe_returns_staged_with_the_same_digest",
            |rig| {
                on_each_staging_shape(
                    rig,
                    base::stage_then_observe_returns_staged_with_the_same_digest::<R>,
                );
            },
        )
        .gated(needs_a_staging_channel::<R>),
        Scenario::new(
            "a_staged_coordinate_is_not_readable_by_a_stranger",
            |rig| {
                on_each_staging_shape(
                    rig,
                    base::a_staged_coordinate_is_not_readable_by_a_stranger::<R>,
                );
            },
        )
        .gated(needs_a_staging_channel::<R>),
        Scenario::new("a_stage_grant_observes_the_staged_state_it_made", |rig| {
            on_each_staging_shape(
                rig,
                base::a_stage_grant_observes_the_staged_state_it_made::<R>,
            );
        })
        .gated(needs_a_staging_channel::<R>),
        Scenario::new("a_commit_grant_observes_it_too", |rig| {
            on_each_staging_shape(rig, base::a_commit_grant_observes_it_too::<R>);
        })
        .gated(needs_a_staging_channel::<R>),
        Scenario::new(
            "commit_makes_the_staged_coordinates_readable_with_the_same_digest",
            |rig| {
                on_each_shape(
                    rig,
                    base::commit_makes_the_staged_coordinates_readable_with_the_same_digest::<R>,
                );
            },
        )
        .gated(needs_a_channel::<R>),
        Scenario::new(
            "stage_of_present_same_is_duplicate_and_changes_nothing",
            |rig| {
                on_each_staging_shape(
                    rig,
                    base::stage_of_present_same_is_duplicate_and_changes_nothing::<R>,
                );
            },
        )
        .gated(needs_a_staging_channel::<R>),
        Scenario::new("commit_over_present_different_leaves_old_bytes", |rig| {
            on_each_of(
                rig,
                &conditional_shapes(rig),
                "a channel that does not stage and is a conditional put to run on",
                base::commit_over_present_different_leaves_old_bytes::<R>,
            );
        })
        .gated(needs_a_conditional_commit::<R>),
        Scenario::new(
            "abort_gives_staged_state_back_and_never_touches_a_committed_coordinate",
            |rig| {
                on_each_staging_shape(
                    rig,
                    base::abort_gives_staged_state_back_and_never_touches_a_committed_coordinate::<R>,
                );
            },
        )
        .gated(needs_a_staging_channel::<R>),
        Scenario::new("a_set_commit_makes_every_coordinate_readable_or_none", |rig| {
            on_each_shape(
                rig,
                base::a_set_commit_makes_every_coordinate_readable_or_none::<R>,
            );
        })
        .gated(needs_a_channel::<R>),
        Scenario::new("a_channel_that_does_not_stage_commits_with_the_bytes", |rig| {
            on_each_unstaged_shape(
                rig,
                base::a_channel_that_does_not_stage_commits_with_the_bytes::<R>,
            );
        })
        .gated(needs_an_unstaged_channel::<R>),
        Scenario::new("no_digest_exposed_is_refused", |rig| {
            on_each_shape(rig, base::no_digest_exposed_is_refused::<R>);
        })
        .gated(needs_a_channel::<R>),
        Scenario::new("yank_keeps_bytes_fetchable", |rig| {
            on_each_shape(rig, base::yank_keeps_bytes_fetchable::<R>);
        })
        .gated(needs_a_channel::<R>),
        Scenario::new("yanked_coordinate_cannot_be_committed_again", |rig| {
            on_each_shape(rig, base::yanked_coordinate_cannot_be_committed_again::<R>);
        })
        .gated(needs_a_channel::<R>),
        Scenario::new("lost_answer_is_resolvable_by_observe", |rig| {
            on_each_shape(rig, base::lost_answer_is_resolvable_by_observe::<R>);
        })
        .gated(needs_a_channel::<R>),
        Scenario::new("list_may_lag_but_observe_is_exact", |rig| {
            on_each_shape(rig, base::list_may_lag_but_observe_is_exact::<R>);
        })
        .gated(needs_a_channel::<R>),
        Scenario::new("public_read_needs_no_grant", |rig| {
            on_each_of(
                rig,
                &public_shapes(rig),
                "a public channel to run on",
                access::public_read_needs_no_grant::<R>,
            );
        })
        .gated(needs_a_public_channel::<R>),
        Scenario::new(
            "a_grant_for_another_repository_sends_nothing",
            access::a_grant_for_another_repository_sends_nothing::<R>,
        )
        .gated(needs_a_channel::<R>),
        Scenario::new(
            "a_grant_of_the_wrong_phase_is_denied_and_nothing_happens",
            access::a_grant_of_the_wrong_phase_is_denied_and_nothing_happens::<R>,
        )
        .gated(needs_a_channel::<R>),
        Scenario::new(
            "an_expired_grant_is_refused_and_nothing_happens",
            access::an_expired_grant_is_refused_and_nothing_happens::<R>,
        )
        .gated(needs_a_channel::<R>),
        Scenario::new("newest_lists_every_held_coordinate_of_the_name", |rig| {
            on_each_shape(rig, access::newest_lists_every_held_coordinate_of_the_name::<R>);
        })
        .gated(needs_a_channel::<R>),
        Scenario::new("observe_returns_the_stamp_it_was_given", |rig| {
            on_each_shape(rig, access::observe_returns_the_stamp_it_was_given::<R>);
        })
        .gated(needs_a_channel::<R>),
        Scenario::new(
            "a_granted_observe_sees_a_coordinate_committed_by_another_writer",
            |rig| {
                on_each_shape(
                    rig,
                    access::a_granted_observe_sees_a_coordinate_committed_by_another_writer::<R>,
                );
            },
        )
        .gated(needs_a_channel::<R>),
        Scenario::new("a_private_observe_reads_the_group_then_the_index", |rig| {
            on_each_of(
                rig,
                &grouped_shapes(rig),
                "a channel addressed by group to run on",
                private::a_private_observe_reads_the_group_then_the_index::<R>,
            );
        })
        .gated(needs_a_grouped_channel::<R>),
        Scenario::new(
            "a_coordinate_committed_in_another_group_reads_as_present_never_absent",
            |rig| {
                on_each_of(
                    rig,
                    &grouped_shapes(rig),
                    "a channel addressed by group to run on",
                    private::a_coordinate_committed_in_another_group_reads_as_present_never_absent::<R>,
                );
            },
        )
        .gated(needs_a_grouped_channel::<R>),
        Scenario::new(
            "a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent",
            |rig| {
                on_each_of(
                    rig,
                    &grouped_shapes(rig),
                    "a channel addressed by group to run on",
                    private::a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent::<R>,
                );
            },
        )
        .gated(needs_a_grouped_channel::<R>),
        Scenario::new(
            "observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set",
            |rig| {
                on_each_of(
                    rig,
                    &grouped_shapes(rig),
                    "a channel addressed by group to run on",
                    private::observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set::<R>,
                );
            },
        )
        .gated(needs_a_grouped_channel::<R>),
        Scenario::new(
            "observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates",
            |rig| {
                on_each_of(
                    rig,
                    &grouped_shapes(rig),
                    "a channel addressed by group to run on",
                    private::observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates::<R>,
                );
            },
        )
        .gated(needs_a_grouped_channel::<R>),
        Scenario::new(
            "a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit",
            |rig| {
                on_each_of(
                    rig,
                    &private_shapes(rig),
                    "a private channel to run on",
                    private::a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit::<R>,
                );
            },
        )
        .gated(needs_a_private_channel::<R>),
        Scenario::new(
            "commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit",
            |rig| {
                on_each_staging_shape(
                    rig,
                    access::commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit::<R>,
                );
            },
        )
        .gated(needs_a_staging_channel::<R>),
        Scenario::new("transient_after_commit_is_observed_before_retry", |rig| {
            on_each_staging_shape(
                rig,
                access::transient_after_commit_is_observed_before_retry::<R>,
            );
        })
        .gated(needs_a_staging_channel::<R>),
        Scenario::new("duplicate_over_other_bytes_leaves_the_old_bytes", |rig| {
            on_each_staging_shape(
                rig,
                access::duplicate_over_other_bytes_leaves_the_old_bytes::<R>,
            );
        })
        .gated(needs_a_staging_channel::<R>),
        Scenario::new("a_second_commit_of_the_same_bytes_is_duplicate", |rig| {
            on_each_shape(
                rig,
                base::a_second_commit_of_the_same_bytes_is_duplicate::<R>,
            );
        })
        .gated(needs_a_channel::<R>),
        Scenario::new("a_stage_over_ones_own_staged_coordinate_is_duplicate", |rig| {
            on_each_staging_shape(
                rig,
                base::a_stage_over_ones_own_staged_coordinate_is_duplicate::<R>,
            );
        })
        .gated(needs_a_staging_channel::<R>),
        Scenario::new("a_commit_carries_exactly_what_was_staged", |rig| {
            on_each_staging_shape(rig, more::a_commit_carries_exactly_what_was_staged::<R>);
        })
        .gated(needs_a_staging_channel::<R>),
        Scenario::new(
            "a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was",
            |rig| {
                on_each_of(
                    rig,
                    &set_commit_shapes(rig),
                    "a channel that commits a set at once to run on",
                    more::a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was::<R>,
                );
            },
        )
        .gated(needs_a_set_commit::<R>),
        Scenario::new("a_set_that_names_a_coordinate_twice_is_refused", |rig| {
            on_each_shape(rig, more::a_set_that_names_a_coordinate_twice_is_refused::<R>);
        })
        .gated(needs_a_channel::<R>),
        Scenario::new(
            "a_stage_or_commit_grant_reads_no_staged_state_but_through_observe",
            more::a_stage_or_commit_grant_reads_no_staged_state_but_through_observe::<R>,
        )
        .gated(needs_a_staging_channel::<R>),
        Scenario::new(
            "a_channel_that_does_not_stage_creates_a_coordinate_once",
            |rig| {
                on_each_unstaged_shape(
                    rig,
                    more::a_channel_that_does_not_stage_creates_a_coordinate_once::<R>,
                );
            },
        )
        .gated(needs_an_unstaged_channel::<R>),
        Scenario::new(
            "a_commit_of_staged_state_that_vanished_is_not_committed_and_a_restage_works",
            |rig| {
                on_each_staging_shape(
                    rig,
                    shapes::a_commit_of_staged_state_that_vanished_is_not_committed_and_a_restage_works::<R>,
                );
            },
        )
        .gated(needs_a_staging_channel::<R>),
        Scenario::new(
            "a_retry_of_a_commit_that_is_becoming_readable_is_committed_or_duplicate_and_never_a_second_write",
            |rig| {
                on_each_shape(
                    rig,
                    shapes::a_retry_of_a_commit_that_is_becoming_readable_is_committed_or_duplicate_and_never_a_second_write::<R>,
                );
            },
        )
        .gated(needs_a_channel::<R>),
        Scenario::new(
            "a_coordinate_the_channel_cannot_hold_is_refused_before_anything_is_sent",
            |rig| {
                on_each_shape(
                    rig,
                    shapes::a_coordinate_the_channel_cannot_hold_is_refused_before_anything_is_sent::<R>,
                );
            },
        )
        .gated(needs_an_unrepresentable_coordinate::<R>),
        Scenario::new(
            "a_sealed_version_refuses_a_later_platform_and_observes_it_absent",
            |rig| {
                on_each_shape(
                    rig,
                    shapes::a_sealed_version_refuses_a_later_platform_and_observes_it_absent::<R>,
                );
            },
        )
        .gated(needs_a_sealing_registry::<R>),
        Scenario::new(
            "a_burned_coordinate_is_neither_absent_nor_present_nor_committable",
            |rig| {
                on_each_shape(
                    rig,
                    shapes::a_burned_coordinate_is_neither_absent_nor_present_nor_committable::<R>,
                );
            },
        )
        .gated(needs_a_registry_that_can_burn::<R>),
        Scenario::new(
            "a_registry_that_echoes_the_declared_digest_fails_readback",
            |rig| {
                on_each_shape(
                    rig,
                    shapes::a_registry_that_echoes_the_declared_digest_fails_readback::<R>,
                );
            },
        )
        .gated(needs_a_channel::<R>),
        Scenario::new(
            "the_declarations_of_a_rig_hold_of_what_it_builds",
            shapes::the_declarations_of_a_rig_hold_of_what_it_builds::<R>,
        ),
    ]
}
