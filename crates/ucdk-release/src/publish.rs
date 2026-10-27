// SPDX-License-Identifier: Apache-2.0
//! The decision to publish one coordinate, from what a registry reports of it.

pub use ucdk_release_proto::v1::Held;
use ucdk_release_proto::v1::Reason;

use crate::transitions::Stand;

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
    /// The coordinate was claimed and can never be published: the number of a group that failed.
    /// It holds no bytes, so there is no digest to compare and no `Held` to carry. It is not
    /// `Absent` (nothing may be put there) and not a withdrawal (nothing was ever published).
    Burned,
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
/// never equality (`CHANNEL`). A yanked coordinate is refused whatever its bytes, and a burned
/// one is refused with `PUBLISH_BURNED`: it is never put, never retried and never read as absent.
///
/// This is the decision of the preflight observation and of the stage phase. The commit phase
/// reads a staged coordinate with another digest as `PUBLISH_STAGE`, because there the staged
/// state is one this action made and it has changed.
pub fn decide_publish(ours: &str, presence: &Presence) -> PublishDecision {
    match presence {
        Presence::Absent => PublishDecision::Put,
        Presence::Yanked(_) => PublishDecision::Refused(Reason::PublishYanked),
        Presence::Burned => PublishDecision::Refused(Reason::PublishBurned),
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

/// Decides the coordinate as [`decide_publish`] does, given also the state of the `publication`
/// record this ledger holds for it. A registry is the truth about bytes and the ledger is the
/// truth about what was done, so the two are put side by side: a record that is `Confirmed`
/// means the coordinate was committed and read back, so a coordinate that now reads `Absent`, or
/// only `Staged`, went away out of band, and the answer is `PUBLISH_VANISHED`, never `Put` and
/// never a commit. A confirmed publication is audit data (3.3): it is neither written again nor
/// reopened, so the bytes are not put back behind it. Every other pairing is the decision of
/// the registry's report alone, as an intent, a missing record or a publication being removed
/// all leave the registry to say what is there.
pub fn decide_publish_recorded(ours: &str, record: Stand, presence: &Presence) -> PublishDecision {
    if record == Stand::Confirmed && matches!(presence, Presence::Absent | Presence::Staged(_)) {
        return PublishDecision::Refused(Reason::PublishVanished);
    }
    decide_publish(ours, presence)
}
