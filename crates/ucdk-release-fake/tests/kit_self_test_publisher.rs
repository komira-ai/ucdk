// SPDX-License-Identifier: Apache-2.0
//! The self-test of the publisher kit, part two: the stubs that take a call the conforming
//! registry refuses, or answer one differently, each around the fake and each named with the
//! scenarios that fail it, and the stub of each scenario that the kit's second part adds.

mod rigs;

use rigs::publisher_more::{Defect, Field, More, MoreRig, Verb, field_blind, on_shapes};
use rigs::{each_fails_alone, further_stubs_fail_exactly, names};
use ucdk_release_seams::kit::{publisher_scenarios, run_publisher_kit};

/// How many scenarios the first part of the kit has: `kit_self_test` holds a stub for each.
const FIRST_PART: usize = 34;

const OBSERVE_ABSENT: &str = "observe_absent_is_not_an_error";
const ABORT: &str = "abort_gives_staged_state_back_and_never_touches_a_committed_coordinate";
const YANK: &str = "yank_keeps_bytes_fetchable";
const FOREIGN: &str = "a_grant_for_another_repository_sends_nothing";
const PHASE: &str = "a_grant_of_the_wrong_phase_is_denied_and_nothing_happens";
const EXPIRED: &str = "an_expired_grant_is_refused_and_nothing_happens";
const PRIVATE: &str =
    "a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit";
const COMMIT_CARRIES: &str = "a_commit_carries_exactly_what_was_staged";
const DUPLICATE_SET: &str = "a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was";
const NAMED_TWICE: &str = "a_set_that_names_a_coordinate_twice_is_refused";
const WRITER_READS: &str = "a_stage_or_commit_grant_reads_no_staged_state_but_through_observe";
const NOT_STAGING: &str = "a_channel_that_does_not_stage_creates_a_coordinate_once";

/// The scenarios that fail the stub, in the kit's order.
fn held_by(stub: More) -> &'static [&'static str] {
    match stub {
        More::GrantBlindToField(..) | More::AbortTakesCommitGrant => &[PHASE],
        More::BadGrantTakenOnShape(_, defect, verb) => match (verb, defect) {
            (Verb::Observe | Verb::Fetch, _) => &[PRIVATE],
            (_, Defect::Foreign) => &[FOREIGN],
            (_, Defect::Expired) => &[EXPIRED],
            (_, Defect::Uncovered | Defect::OtherWritePhase) => &[PHASE],
        },
        More::CommitReplacesStaged | More::CommitUnstaged => &[COMMIT_CARRIES],
        More::CommitPartial | More::StagePartial => &[DUPLICATE_SET],
        More::DupStageDeduped | More::DupCommitDeduped | More::DupObserveSetAccepted => {
            &[NAMED_TWICE]
        }
        More::FetchWithWriterGrantSeesStaged
        | More::NewestWithWriterGrantIncludesStaged
        | More::ListWithWriterGrantIncludesStaged => &[WRITER_READS],
        More::NonStagingRaceOverwrites
        | More::NonStagingCommitOverYanked
        | More::NonStagingTransientIsPermanent
        | More::NonStagingSetCommitIsTorn => &[NOT_STAGING],
        More::YankAbsentOk | More::YankSiblings(_) => &[YANK],
        More::StageAfterAbortIsDuplicate | More::AbortSiblings(_) => &[ABORT],
        // The abort scenario stages two siblings and gives one back, and finds out these two too.
        More::StageSiblingTaken(_) | More::ObserveStagedSibling(_) => &[OBSERVE_ABSENT, ABORT],
        More::CommitSiblingTaken(_) => &[OBSERVE_ABSENT],
    }
}

#[test]
fn each_scenario_of_the_second_part_fails_on_its_nonconforming_stub() {
    each_fails_alone(
        "publisher (second part)",
        &names(&publisher_scenarios::<MoreRig>())[FIRST_PART..],
        &[
            (COMMIT_CARRIES, More::CommitReplacesStaged),
            (DUPLICATE_SET, More::CommitPartial),
            (NAMED_TWICE, More::DupStageDeduped),
            (WRITER_READS, More::FetchWithWriterGrantSeesStaged),
            (NOT_STAGING, More::NonStagingRaceOverwrites),
        ],
        |flaw| run_publisher_kit(&MoreRig::flawed(flaw)),
    );
}

#[test]
fn each_further_publisher_stub_of_part_two_fails_the_scenarios_that_hold_it() {
    let mut stubs: Vec<More> = field_blind();
    stubs.extend(on_shapes());
    stubs.extend([
        More::CommitReplacesStaged,
        More::CommitUnstaged,
        More::CommitPartial,
        More::StagePartial,
        More::FetchWithWriterGrantSeesStaged,
        More::NewestWithWriterGrantIncludesStaged,
        More::ListWithWriterGrantIncludesStaged,
        More::NonStagingRaceOverwrites,
        More::NonStagingCommitOverYanked,
        More::NonStagingTransientIsPermanent,
        More::NonStagingSetCommitIsTorn,
        More::AbortTakesCommitGrant,
        More::YankAbsentOk,
        More::StageAfterAbortIsDuplicate,
        More::DupStageDeduped,
        More::DupCommitDeduped,
        More::DupObserveSetAccepted,
    ]);
    for field in Field::SIBLING {
        stubs.extend([
            More::YankSiblings(field),
            More::AbortSiblings(field),
            More::StageSiblingTaken(field),
            More::CommitSiblingTaken(field),
            More::ObserveStagedSibling(field),
        ]);
    }
    let table: Vec<(More, &[&'static str])> = stubs
        .into_iter()
        .map(|stub| (stub, held_by(stub)))
        .collect();
    further_stubs_fail_exactly(
        "publisher",
        &names(&publisher_scenarios::<MoreRig>()),
        &table,
        |flaw| run_publisher_kit(&MoreRig::flawed(flaw)),
    );
}
