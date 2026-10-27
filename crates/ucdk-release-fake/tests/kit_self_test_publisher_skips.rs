// SPDX-License-Identifier: Apache-2.0
//! The skips of the publisher kit are held to what a rig declares: a rig of one channel that
//! stages and is addressed by group, private or public, runs every scenario but the ones whose
//! needed capability it declares absent, and a gate that names a capability the rig declares is
//! a defect that is found.

mod rigs;

use rigs::names;
use rigs::publisher::FakePublisherRig;
use rigs::publisher_names::*;
use ucdk_release_seams::kit::{
    ChannelShape, Need, Report, Scenario, Verdict, publisher_scenarios, run_publisher_kit,
    skips_that_name_a_declared_capability,
};

/// What a report skipped, in the kit's order, with the capability each scenario needed.
fn skipped_with_their_needs(report: &Report) -> Vec<(&'static str, Need)> {
    report
        .outcomes
        .iter()
        .filter_map(|o| match o.verdict {
            Verdict::Skipped(need) => Some((o.name, need)),
            _ => None,
        })
        .collect()
}

/// Every scenario of the kit but `skipped`, in the kit's order.
fn all_but(skipped: &[(&'static str, Need)]) -> Vec<&'static str> {
    names(&publisher_scenarios::<FakePublisherRig>())
        .into_iter()
        .filter(|n| !skipped.iter().any(|(s, _)| s == n))
        .collect()
}

/// A rig of one private channel that stages and is addressed by group declares no public channel,
/// no channel that does not stage (so no conditional put and no commit that carries the bytes) and
/// no registry that seals a version. It runs every other scenario, the lifecycle scenarios (a
/// race, a duplicate, a set, an abort, a yank, `newest`, a stamp, a read-back, a second commit, a
/// retry, a vanished reservation, a refused coordinate, a burned number) included, and skips only
/// those five, each for the capability it lacks.
#[test]
fn a_private_channel_addressed_by_group_runs_every_scenario_but_five() {
    let rig = FakePublisherRig::conforming().with_shapes(vec![ChannelShape::private_grouped()]);
    let report = run_publisher_kit(&rig);
    report.assert_conforms();
    let skipped = [
        (COMMIT_OVER, Need::ConditionalCommit),
        (NOT_STAGE_BYTES, Need::UnstagedChannel),
        (PUBLIC_READ, Need::PublicChannel),
        (NOT_STAGING, Need::UnstagedChannel),
        (SEALED, Need::SealingRegistry),
    ];
    assert_eq!(skipped_with_their_needs(&report), skipped);
    assert_eq!(report.passed(), all_but(&skipped));
    assert_eq!(report.passed().len(), 41);
    assert_eq!(report.names().len(), 46);
}

/// A rig of one public channel that stages, is addressed by group, commits a set and yanks a
/// version declares no private channel, no channel that does not stage and no registry that seals
/// a version, and skips only the five scenarios that need one of them.
#[test]
fn a_public_channel_addressed_by_group_runs_every_scenario_but_five() {
    let rig =
        FakePublisherRig::yanking_a_version().with_shapes(vec![ChannelShape::public_grouped()]);
    let report = run_publisher_kit(&rig);
    report.assert_conforms();
    let skipped = [
        (COMMIT_OVER, Need::ConditionalCommit),
        (NOT_STAGE_BYTES, Need::UnstagedChannel),
        (READ_GRANT, Need::PrivateChannel),
        (NOT_STAGING, Need::UnstagedChannel),
        (SEALED, Need::SealingRegistry),
    ];
    assert_eq!(skipped_with_their_needs(&report), skipped);
    assert_eq!(report.passed(), all_but(&skipped));
    assert_eq!(report.passed().len(), 41);
    assert_eq!(report.names().len(), 46);
}

fn nothing(_: &FakePublisherRig) {}

fn names_staging(_: &FakePublisherRig) -> Option<Need> {
    Some(Need::StagingChannel)
}

fn names_group_addressing(_: &FakePublisherRig) -> Option<Need> {
    Some(Need::GroupedChannel)
}

fn names_a_private_channel(_: &FakePublisherRig) -> Option<Need> {
    Some(Need::PrivateChannel)
}

/// The one stub of the check: a gate that mislabels the shape of the rig it runs on. The rig
/// declares a channel that stages and is addressed by group, and the gates of two scenarios say it
/// declares neither; both are found, and the gate that names the private channel the rig does not
/// declare is not.
#[test]
fn a_skip_that_names_a_capability_the_rig_declares_is_found() {
    let rig = FakePublisherRig::conforming().with_shapes(vec![ChannelShape::public_grouped()]);
    let kit = vec![
        Scenario::new("mislabels_staging", nothing).gated(names_staging),
        Scenario::new("mislabels_group_addressing", nothing).gated(names_group_addressing),
        Scenario::new("names_what_is_absent", nothing).gated(names_a_private_channel),
        Scenario::new("is_not_gated", nothing),
    ];
    assert_eq!(
        skips_that_name_a_declared_capability(&rig, &kit),
        vec![
            ("mislabels_staging", Need::StagingChannel),
            ("mislabels_group_addressing", Need::GroupedChannel),
        ]
    );
}

/// The gates of the kit name only what the rigs of the self-test declare absent: on a rig of every
/// shape, of one shape, and of none.
#[test]
fn the_gates_of_the_kit_name_only_capabilities_a_rig_declares_absent() {
    let rigs = [
        FakePublisherRig::conforming(),
        FakePublisherRig::restricted(),
        FakePublisherRig::conforming().with_shapes(vec![ChannelShape::private_grouped()]),
        FakePublisherRig::conforming().with_shapes(vec![ChannelShape::public_grouped()]),
        FakePublisherRig::conforming().with_shapes(Vec::new()),
        FakePublisherRig::unconditional(),
        FakePublisherRig::per_coordinate(),
        FakePublisherRig::unable_to_burn(),
        FakePublisherRig::holding_every_coordinate(),
        FakePublisherRig::sealing(),
    ];
    for rig in &rigs {
        assert_eq!(
            skips_that_name_a_declared_capability(rig, &publisher_scenarios()),
            Vec::new()
        );
    }
}
