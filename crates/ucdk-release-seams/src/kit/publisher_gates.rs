// SPDX-License-Identifier: Apache-2.0
//! What a scenario of the publisher kit needs of a rig, as a typed [`Need`], and the gates that
//! answer it. A gate answers the capability the rig declares absent and never the shape of a
//! channel: a scenario about what a channel does runs on every declared shape that can do it, and
//! is skipped only by a rig that declares none (`stages`, `commit_unit`, `private`,
//! `group_addressed`, the hooks `unrepresentable`, `seals_versions` and `can_burn`).

use super::publisher_rig::{PublisherRig, built};
use super::{Need, Scenario};
use crate::{CommitUnit, Publisher};

/// Whether `rig` declares what `need` names.
fn declares<R: PublisherRig>(rig: &R, need: Need) -> bool {
    let shapes = rig.shapes();
    match need {
        Need::Channel => !shapes.is_empty(),
        Need::PublicChannel => shapes.iter().any(|s| !s.private),
        Need::PrivateChannel => shapes.iter().any(|s| s.private),
        Need::GroupedChannel => shapes.iter().any(|s| s.group_addressed && s.stages),
        Need::StagingChannel => shapes.iter().any(|s| s.stages),
        Need::UnstagedChannel => shapes.iter().any(|s| !s.stages),
        Need::SetCommit => shapes
            .iter()
            .any(|s| built(rig, s).commit_unit() == CommitUnit::Set),
        Need::ConditionalCommit => shapes
            .iter()
            .any(|s| !s.stages && built(rig, s).conditional_put()),
        Need::UnrepresentableCoordinate => !rig.unrepresentable().is_empty(),
        Need::SealingRegistry => rig.seals_versions(),
        Need::BurningRegistry => rig.can_burn(),
    }
}

/// `need`, when the rig does not declare it.
fn lacks<R: PublisherRig>(rig: &R, need: Need) -> Option<Need> {
    (!declares(rig, need)).then_some(need)
}

/// The gate of a scenario that runs on a channel of any shape.
pub(super) fn needs_a_channel<R: PublisherRig>(rig: &R) -> Option<Need> {
    lacks(rig, Need::Channel)
}

/// The gate of a scenario about what a stranger reads of a public channel.
pub(super) fn needs_a_public_channel<R: PublisherRig>(rig: &R) -> Option<Need> {
    lacks(rig, Need::PublicChannel)
}

/// The gate of a scenario about a channel whose every read needs a grant.
pub(super) fn needs_a_private_channel<R: PublisherRig>(rig: &R) -> Option<Need> {
    lacks(rig, Need::PrivateChannel)
}

/// The gate of a scenario about staged state addressed by group.
pub(super) fn needs_a_grouped_channel<R: PublisherRig>(rig: &R) -> Option<Need> {
    lacks(rig, Need::GroupedChannel)
}

/// The gate of a scenario about staged state, on a channel of any shape that stages.
pub(super) fn needs_a_staging_channel<R: PublisherRig>(rig: &R) -> Option<Need> {
    lacks(rig, Need::StagingChannel)
}

/// The gate of a scenario about a channel whose commit carries the bytes, public or private.
pub(super) fn needs_an_unstaged_channel<R: PublisherRig>(rig: &R) -> Option<Need> {
    lacks(rig, Need::UnstagedChannel)
}

/// The gate of a scenario that commits several coordinates in one call, on any shape.
pub(super) fn needs_a_set_commit<R: PublisherRig>(rig: &R) -> Option<Need> {
    lacks(rig, Need::SetCommit)
}

/// The gate of a scenario about a conditional put on a channel whose commit carries the bytes.
pub(super) fn needs_a_conditional_commit<R: PublisherRig>(rig: &R) -> Option<Need> {
    lacks(rig, Need::ConditionalCommit)
}

/// The gate of a scenario about a coordinate the channel cannot hold.
pub(super) fn needs_an_unrepresentable_coordinate<R: PublisherRig>(rig: &R) -> Option<Need> {
    needs_a_channel(rig).or_else(|| lacks(rig, Need::UnrepresentableCoordinate))
}

/// The gate of a scenario about a version the registry seals.
pub(super) fn needs_a_sealing_registry<R: PublisherRig>(rig: &R) -> Option<Need> {
    needs_a_channel(rig).or_else(|| lacks(rig, Need::SealingRegistry))
}

/// The gate of a scenario about a number the registry burns.
pub(super) fn needs_a_registry_that_can_burn<R: PublisherRig>(rig: &R) -> Option<Need> {
    needs_a_channel(rig).or_else(|| lacks(rig, Need::BurningRegistry))
}

/// The scenarios of `scenarios` that `rig` skips for a capability it declares, each with the
/// need its gate named. Each of the kit's own gates answers `lacks(rig, need)`, which computes
/// `declares` from the same declarations, so for those gates this comparison cannot fail: a gate
/// that names the wrong `Need` is found only by the run and skip sets the self-test pins. The
/// comparison finds a mislabel on the rig's side: a skip that a gate supplied beside the kit's
/// labels with a capability the rig declares. Empty when every skip names a capability the rig
/// declares absent.
pub fn skips_that_name_a_declared_capability<R: PublisherRig>(
    rig: &R,
    scenarios: &[Scenario<R>],
) -> Vec<(&'static str, Need)> {
    scenarios
        .iter()
        .filter_map(|s| s.skip_need(rig).map(|need| (s.name, need)))
        .filter(|(_, need)| declares(rig, *need))
        .collect()
}
