// SPDX-License-Identifier: Apache-2.0
//! The decision to publish one coordinate, from what a registry reports of it.

pub use ucdk_release_proto::v1::Held;
use ucdk_release_proto::v1::Reason;

/// What a registry reports for a coordinate, to an observer that carries a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    /// Nothing is published or staged there.
    Absent,
    /// The registry holds it staged: not readable by a stranger and not yet published, so only
    /// an observe that carries a grant can return it.
    Staged(Held),
    /// It is published, with what the registry holds.
    Present(Held),
    /// It was published and then withdrawn: burned, whatever its bytes. A registry may report a
    /// coordinate withdrawn by someone else.
    Yanked(Held),
}

/// What to do with one coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishDecision {
    /// Nothing is there: stage it and commit it, or commit it on a channel that does not stage.
    Put,
    /// Our bytes are staged and not yet published: go on to the commit, stage nothing.
    AlreadyStaged,
    /// Our bytes are already published: confirm, write nothing.
    AlreadyPresent,
    /// Never write, never retry, never force.
    Refused(Reason),
}

/// Decides the coordinate whose bytes have the identity digest `ours`, given what the registry
/// reports. Digests are compared byte for byte. A held coordinate, staged or published, that
/// exposes no digest cannot be compared, and a comparison with no common field is a refusal,
/// never equality (`CHANNEL`). A yanked coordinate is refused whatever its bytes.
///
/// This is the decision of the preflight observation and of the stage phase. The commit phase
/// reads a staged coordinate with another digest as `PUBLISH_STAGE`, because there the staged
/// state is one this action made and it has changed.
pub fn decide_publish(ours: &str, presence: &Presence) -> PublishDecision {
    match presence {
        Presence::Absent => PublishDecision::Put,
        Presence::Yanked(_) => PublishDecision::Refused(Reason::PublishYanked),
        Presence::Staged(held) | Presence::Present(held) => {
            if held.digest.is_empty() {
                PublishDecision::Refused(Reason::Channel)
            } else if held.digest != ours {
                PublishDecision::Refused(Reason::PublishDifferentBytes)
            } else if matches!(presence, Presence::Staged(_)) {
                PublishDecision::AlreadyStaged
            } else {
                PublishDecision::AlreadyPresent
            }
        }
    }
}
