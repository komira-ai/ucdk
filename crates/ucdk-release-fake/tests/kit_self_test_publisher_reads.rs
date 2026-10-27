// SPDX-License-Identifier: Apache-2.0
//! The self-test of the publisher kit for what a read gives away and what a call sends: a stranger
//! told the declared set who sees staged state, a private channel that refuses only what it holds
//! and so tells a stranger which packages exist, an observe told no set that reads the group, a
//! publisher that sends a call whose grant it can see is bad, a count of the requests sent that
//! never moves or leaves out the refused ones, a channel addressed by group that drops staged state
//! when it refuses an abort, a rig whose only channel that stages
//! is private or addressed by group, a private channel that refuses the reads of a stage grant, and
//! a registry that takes a sibling of a coordinate only together with it.

mod rigs;

use rigs::publisher::{FakePublisherRig, Flaw};
use rigs::publisher_holes::{Hole, HoleRig, Whose, refusing_only_what_is_held};
use rigs::publisher_lax::{Defect, Kind};
use rigs::publisher_more::{More, MoreRig};
use rigs::publisher_names::*;
use rigs::{further_stubs_fail_exactly, names};
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::kit::{
    ChannelShape, PublisherRig, Report, publisher_scenarios, run_publisher_kit,
};
use ucdk_release_seams::{ListScope, Publisher, SeamError, StageItem};

#[derive(Debug, Clone, Copy)]
enum Stub {
    Hole(Hole),
    Flaw(Flaw),
}

fn run(stub: Stub) -> Report {
    match stub {
        Stub::Hole(hole) => run_publisher_kit(&HoleRig::new(hole)),
        Stub::Flaw(flaw) => run_publisher_kit(&FakePublisherRig::flawed(flaw)),
    }
}

/// Every stub with the scenarios that fail it, in the kit's order.
fn table() -> Vec<(Stub, &'static [&'static str])> {
    let mut all: Vec<(Stub, &'static [&'static str])> = vec![
        // A stranger told the declared set is asked of every public channel that stages.
        (
            Stub::Hole(Hole::StrangerToldTheSetSeesStaged(Kind::Public)),
            &[PUBLIC_READ],
        ),
        (
            Stub::Hole(Hole::StrangerToldTheSetSeesStaged(Kind::Grouped)),
            &[PUBLIC_READ],
        ),
        // An observe told no set reads the index alone.
        (Stub::Hole(Hole::EmptySetReadsTheGroup), &[EMPTY_SET]),
        // A publisher that sends a call whose grant is bad sends it with the secret of a grant the
        // registry was not meant to see. The three kinds of bad grant are asked of the reads of a
        // public and of a private channel and of the writes of every channel, and the grant of
        // another phase, which a read does not have, of the writes only.
        (
            Stub::Flaw(Flaw::SendsRefused(Defect::Foreign)),
            &[PUBLIC_READ, FOREIGN, READ_GRANT],
        ),
        (
            Stub::Flaw(Flaw::SendsRefused(Defect::Expired)),
            &[PUBLIC_READ, EXPIRED, READ_GRANT],
        ),
        (
            Stub::Flaw(Flaw::SendsRefused(Defect::Uncovered)),
            &[PUBLIC_READ, PHASE, READ_GRANT],
        ),
        (
            Stub::Flaw(Flaw::SendsRefused(Defect::OtherWritePhase)),
            &[PHASE],
        ),
        // The count of requests sent is held to advance: a count that is always zero and a count
        // of the admitted requests alone are failed by the scenario about what a rig declares.
        (Stub::Hole(Hole::SentIsAlwaysZero), &[DECLARES]),
        (Stub::Hole(Hole::SentCountsOnlyAdmitted), &[DECLARES]),
        // A channel addressed by group that refuses an abort and drops the staged state all the
        // same is told by an observe of that state, told the set, after the refusal.
        (
            Stub::Hole(Hole::RefusedAbortDropsStaged(Whose::Stranger)),
            &[PHASE],
        ),
        (
            Stub::Hole(Hole::RefusedAbortDropsStaged(Whose::PartialGrant)),
            &[PHASE],
        ),
    ];
    // A private channel that refuses only what it holds tells a stranger, and a bad grant, which
    // packages exist.
    all.extend(
        refusing_only_what_is_held()
            .into_iter()
            .map(|hole| (Stub::Hole(hole), &[READ_GRANT][..])),
    );
    all
}

/// Every stub of what a read gives away or a call sends fails the scenarios that hold it, and no
/// other.
#[test]
fn each_stub_of_what_a_read_gives_away_or_a_call_sends_fails_the_scenarios_that_hold_it() {
    let table = table();
    assert_eq!(table.len(), 23,"the number of stubs of the table");
    further_stubs_fail_exactly(
        "publisher",
        &names(&publisher_scenarios::<FakePublisherRig>()),
        &table,
        run,
    );
}

/// The channels a rig declares alone, none of them a public one addressed by coordinate: a private
/// channel addressed by group, a public one addressed by group and a private one addressed by
/// coordinate.
fn the_only_channel_that_stages() -> [ChannelShape; 3] {
    [
        ChannelShape::private_grouped(),
        ChannelShape::public_grouped(),
        ChannelShape {
            group_addressed: false,
            ..ChannelShape::private_grouped()
        },
    ]
}

/// A rig whose only channel that stages is private or addressed by group is held to what a stage
/// grant and a commit grant read of staged state: a registry that answers staged bytes to `fetch`,
/// or names staged coordinates in `newest` or `list`, is failed by that scenario and by no other,
/// and the conforming fake passes it there.
#[test]
fn a_rig_with_no_public_channel_that_stages_is_held_to_what_a_stage_grant_reads() {
    for shape in the_only_channel_that_stages() {
        let base = FakePublisherRig::conforming().with_shapes(vec![shape]);
        let report = run_publisher_kit(&base);
        report.assert_conforms();
        assert!(report.passed().contains(&WRITER_READS), "{shape:?}");
        for stub in [
            More::FetchWithWriterGrantSeesStaged,
            More::NewestWithWriterGrantIncludesStaged,
            More::ListWithWriterGrantIncludesStaged,
        ] {
            let base = FakePublisherRig::conforming().with_shapes(vec![shape]);
            let failed = run_publisher_kit(&MoreRig::flawed_on(base, stub)).failed();
            assert_eq!(
                failed,
                vec![WRITER_READS],
                "{stub:?} on a rig of {shape:?} alone"
            );
        }
    }
}

/// A stage grant and a commit grant are for `observe`: a private channel may refuse their `fetch`,
/// `newest` and `list` with `Denied` as well as answer nothing, and the kit passes a registry that
/// does. The fake that refuses them does so on a private channel and on no other.
#[test]
fn a_private_channel_that_refuses_the_reads_of_a_stage_grant_passes_the_kit() {
    let rig = FakePublisherRig::refusing_writer_reads();
    let report = run_publisher_kit(&rig);
    report.assert_conforms();
    assert!(report.passed().contains(&WRITER_READS));
    let k = rig.coordinate("a");
    let denied = SeamError::GrantRefused(GrantFault::Denied);
    let mut private = rig.publisher(&ChannelShape::private_grouped());
    let stage_grant = rig.grant(&private, GrantPhase::Stage, std::slice::from_ref(&k));
    assert_eq!(private.fetch(&stage_grant, &k), Err(denied.clone()));
    assert_eq!(
        private.newest(&stage_grant, &k.channel, &k.name),
        Err(denied.clone())
    );
    assert_eq!(
        private.list(
            &stage_grant,
            &ListScope {
                channel: k.channel.clone(),
                prefix: String::new()
            }
        ),
        Err(denied)
    );
    // An observe with the same grant is answered, and so is a read grant's fetch.
    assert!(private.observe(&stage_grant, &k, &[]).is_ok());
    let read_grant = rig.grant(&private, GrantPhase::Read, std::slice::from_ref(&k));
    assert_eq!(private.fetch(&read_grant, &k), Ok(None));
    let mut public = rig.publisher(&ChannelShape::staging());
    let stage_grant = rig.grant(&public, GrantPhase::Stage, std::slice::from_ref(&k));
    assert_eq!(public.fetch(&stage_grant, &k), Ok(None));
}

/// A registry that takes a sibling of a coordinate only together with that coordinate is passed by
/// the kit when the rig says every sibling is published in one set with its base: the scenario about
/// the fields of a coordinate stages a sibling with its base, as it is published. The registry
/// does refuse a sibling that is staged alone.
#[test]
fn a_registry_that_takes_a_sibling_only_with_its_base_passes_the_kit() {
    let rig = FakePublisherRig::pairing();
    let report = run_publisher_kit(&rig);
    report.assert_conforms();
    assert_eq!(
        report.skipped(),
        vec!["a_sealed_version_refuses_a_later_platform_and_observes_it_absent"]
    );
    let base = rig.coordinate("a");
    let siblings = rig.siblings(&base);
    assert!(!siblings.is_empty());
    for sibling in siblings {
        let mut p = rig.publisher(&ChannelShape::staging());
        let payload = rig.payload(&sibling.coordinate, 0);
        let file = ucdk_release_seams::Blob {
            name: &payload.name,
            bytes: &payload.bytes,
        };
        let held = ucdk_release::publish::Held::default();
        let items = [StageItem {
            coordinate: &sibling.coordinate,
            file: &file,
            held: &held,
        }];
        let grant = rig.grant(
            &p,
            GrantPhase::Stage,
            std::slice::from_ref(&sibling.coordinate),
        );
        assert_eq!(
            p.stage(&grant, &items),
            Err(SeamError::Refused(Reason::Channel)),
            "a {} sibling staged alone",
            sibling.field
        );
    }
}
