// SPDX-License-Identifier: Apache-2.0
//! The decision to publish one coordinate, from what a registry reports of it.

pub use ucdk_release_proto::v1::Held;
use ucdk_release_proto::v1::Reason;

/// What a registry reports for a coordinate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Presence {
    /// Nothing is published there.
    Absent,
    /// It is published, with what the registry holds.
    Present(Held),
    /// It was published and then withdrawn: burned, whatever its bytes. A registry may report
    /// a coordinate withdrawn by someone else.
    Yanked(Held),
}

/// What to do with one coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishDecision {
    /// Nothing is there: write it.
    Put,
    /// Our bytes are already there: confirm, write nothing.
    AlreadyPresent,
    /// Never write, never retry, never force.
    Refused(Reason),
}

/// Decides the coordinate whose bytes have the identity digest `ours`, given what the registry
/// reports. A held coordinate that exposes no digest cannot be compared, and a comparison with
/// no common field is a refusal, never equality.
pub fn decide_publish(ours: &str, presence: &Presence) -> PublishDecision {
    match presence {
        Presence::Absent => PublishDecision::Put,
        Presence::Yanked(_) => PublishDecision::Refused(Reason::PublishYanked),
        Presence::Present(held) if held.digest.is_empty() => {
            PublishDecision::Refused(Reason::Channel)
        }
        Presence::Present(held) if held.digest == ours => PublishDecision::AlreadyPresent,
        Presence::Present(_) => PublishDecision::Refused(Reason::PublishDifferentBytes),
    }
}
