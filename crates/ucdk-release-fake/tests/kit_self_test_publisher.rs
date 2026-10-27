// SPDX-License-Identifier: Apache-2.0
//! The self-test of the publisher kit, part two: the stubs that take a call the conforming
//! registry refuses, or answer one differently, each around the fake and each named with the
//! scenarios that fail it.

mod rigs;

use rigs::publisher_more::{Field, More, MoreRig, field_blind, on_shapes};
use rigs::{further_stubs_fail_exactly, names};
use ucdk_release_seams::kit::{publisher_scenarios, run_publisher_kit};

#[test]
fn each_further_publisher_stub_of_part_two_fails_the_scenarios_that_hold_it() {
    let mut table: Vec<(More, &[&'static str])> = Vec::new();
    for stub in field_blind() {
        table.push((stub, &[]));
    }
    for stub in on_shapes() {
        table.push((stub, &[]));
    }
    for stub in [
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
    ] {
        table.push((stub, &[]));
    }
    for field in Field::SIBLING {
        for stub in [
            More::YankSiblings(field),
            More::AbortSiblings(field),
            More::StageSiblingTaken(field),
            More::CommitSiblingTaken(field),
            More::ObserveStagedSibling(field),
        ] {
            table.push((stub, &[]));
        }
    }
    further_stubs_fail_exactly(
        "publisher",
        &names(&publisher_scenarios::<MoreRig>()),
        &table,
        |flaw| run_publisher_kit(&MoreRig::flawed(flaw)),
    );
}
