// SPDX-License-Identifier: Apache-2.0
//! The scenarios of the publisher kit about channels addressed by group and about private
//! channels: how an observe reads staged state by the group of the declared set, what it sees
//! with an empty set, what a coordinate of another writer reads as, and what a read grant and a
//! stranger may do on a channel whose every read needs a grant.

use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::Presence;

use super::publisher_grants::denied;
use super::publisher_reads as reads;
use super::publisher_rig::{
    Arrangement, ChannelShape, PublisherRig, ReadStep, Set, built, held_by_another, observed,
    phases_of, present_held, publish, stage, staged_held,
};
use super::{must, must_err, same};
use crate::{Publisher, SeamError};

/// An observe of a channel addressed by group is a group read and then an index read.
pub(super) fn a_private_observe_reads_the_group_then_the_index<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    let before = rig.reads(&p).len();
    same(
        observed(rig, &mut p, GrantPhase::Stage, &set, 0),
        Presence::Absent,
        "a coordinate nobody wrote",
    );
    same(
        rig.reads(&p)[before..].to_vec(),
        vec![ReadStep::Group, ReadStep::Index],
        "the reads of one observe",
    );
}

/// A coordinate another writer committed, in a group this caller does not know, is read through
/// the index: it is `Present`, never `Absent`, to an observe told the declared set with a grant of
/// any phase, a read grant included.
pub(super) fn a_coordinate_committed_in_another_group_reads_as_present_never_absent<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    rig.arrange(&mut p, &held_by_another(rig, &set.coords[0]));
    for phase in phases_of(shape) {
        same(
            matches!(observed(rig, &mut p, phase, &set, 0), Presence::Present(_)),
            true,
            &format!(
                "a coordinate another writer committed, observed with a {phase:?} grant told the declared set ({shape:?})"
            ),
        );
    }
}

/// A read that fails fails the observe: it is not taken for an absent group or an absent index
/// entry, and it does not promise that nothing happened.
pub(super) fn a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    for (which, failure) in [
        ("group read", Arrangement::FailGroupRead),
        ("index read", Arrangement::FailIndexRead),
    ] {
        let mut p = built(rig, shape);
        rig.arrange(&mut p, &failure);
        let set = Set::of(rig, &["a"]);
        let blobs = set.blobs();
        let items = set.items(&blobs);
        let grant = rig.grant(&p, GrantPhase::Stage, &set.coords);
        let error = must_err(
            p.observe(&grant, &set.coords[0], &items),
            &format!("an observe whose {which} fails"),
        );
        same(
            error.promises_no_effect(),
            false,
            &format!("a failed {which} is Transient or Unknown"),
        );
    }
}

/// Staged state is addressed by the group of the set the caller declares: an observe told the
/// set finds it, and one told a set that does not hold the coordinate is `Refused`.
pub(super) fn observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a", "b"]);
    stage(rig, &mut p, &set);
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    let outside = Set::of(rig, &["a", "b", "c"]);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Stage, &outside.coords);
    let refusal = must_err(
        p.observe(&grant, &outside.coords[2], &items),
        "an observe of a coordinate the declared set does not hold",
    );
    same(
        matches!(refusal, SeamError::Refused(_)),
        true,
        "an observe told a set that does not hold the coordinate is Refused",
    );
}

/// With no set to name a group, an observe reads the index alone: it finds nothing of staged
/// state and still finds what is committed.
pub(super) fn observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let both = Set::of(rig, &["a", "b"]);
    stage(rig, &mut p, &Set::of(rig, &["a"]));
    rig.arrange(&mut p, &held_by_another(rig, &both.coords[1]));
    let grant = rig.grant(&p, GrantPhase::Stage, &both.coords);
    let before = rig.reads(&p).len();
    same(
        must(
            p.observe(&grant, &both.coords[0], &[]),
            "an observe of staged state with no set",
        ),
        Presence::Absent,
        "staged state seen with no set",
    );
    same(
        rig.reads(&p)[before..].to_vec(),
        vec![ReadStep::Index],
        "the reads of an observe told no set: there is no group to read",
    );
    let before = rig.reads(&p).len();
    present_held(must(
        p.observe(&grant, &both.coords[1], &[]),
        "an observe of a committed coordinate with no set",
    ));
    same(
        rig.reads(&p)[before..].to_vec(),
        vec![ReadStep::Index],
        "the reads of an observe of a committed coordinate told no set",
    );
}

/// On a private channel a read grant observes what is committed and is no stage, commit, abort
/// or yank grant, and a stranger reads nothing; a read needs a grant for this repository that
/// names what is read and has not expired.
pub(super) fn a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let (a, b) = (Set::of(rig, &["a"]), Set::of(rig, &["b"]));
    let (staged, theirs) = (Set::of(rig, &["s"]), Set::of(rig, &["o"]));
    publish(rig, &mut p, &a);
    stage(rig, &mut p, &staged);
    rig.arrange(&mut p, &held_by_another(rig, &theirs.coords[0]));
    reads::a_stranger_reads_nothing_of_a_private_channel(rig, shape);
    let read_a = rig.grant(&p, GrantPhase::Read, &a.coords);
    let read_b = rig.grant(&p, GrantPhase::Read, &b.coords);
    present_held(must(
        p.observe(&read_a, &a.coords[0], &[]),
        "an observe with a read grant",
    ));
    let blobs = b.blobs();
    let items = b.items(&blobs);
    let mut refusals = vec![
        ("commit", must_err(p.commit(&read_b, &items), "commit")),
        (
            "yank",
            must_err(p.yank(&read_a, &a.coords[0], "a reason"), "yank"),
        ),
    ];
    if p.stages() {
        refusals.push(("stage", must_err(p.stage(&read_b, &items), "stage")));
        refusals.push(("abort", must_err(p.abort(&read_a, &a.coords), "abort")));
    }
    for (call, refusal) in &refusals {
        same(
            denied(refusal),
            true,
            &format!("a {call} with a read grant is Denied ({shape:?})"),
        );
    }
    same(
        must(
            p.observe(&read_b, &b.coords[0], &[]),
            "an observe after the refusals",
        ),
        Presence::Absent,
        "the coordinate a read grant tried to write",
    );
    present_held(must(
        p.observe(&read_a, &a.coords[0], &[]),
        "an observe after the refusals",
    ));
    reads::a_bad_grant_reads_nothing(rig, shape);
}
