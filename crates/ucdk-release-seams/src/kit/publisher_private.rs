// SPDX-License-Identifier: Apache-2.0
//! The scenarios of the publisher kit about channels addressed by group and about private
//! channels: how an observe reads staged state by the group of the declared set, what it sees
//! with an empty set, what a coordinate of another writer reads as, and what a read grant and a
//! stranger may do on a channel whose every read needs a grant.

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::Presence;

use super::publisher_grants::{denied, is_denied};
use super::publisher_rig::{
    A_DAY_MS, Arrangement, ChannelShape, PublisherRig, ReadStep, Set, built, held_by_another,
    observed, present_held, publish, stage, staged_held,
};
use super::{must, must_err, same};
use crate::{Grant, ListScope, Publisher, SeamError};

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
/// the index: it is `Present`, never `Absent`.
pub(super) fn a_coordinate_committed_in_another_group_reads_as_present_never_absent<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    rig.arrange(&mut p, &held_by_another(rig, &set.coords[0]));
    present_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
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
    same(
        must(
            p.observe(&grant, &both.coords[0], &[]),
            "an observe of staged state with no set",
        ),
        Presence::Absent,
        "staged state seen with no set",
    );
    present_held(must(
        p.observe(&grant, &both.coords[1], &[]),
        "an observe of a committed coordinate with no set",
    ));
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
    a_stranger_reads_nothing_of_a_private_channel(&mut p, &[&a, &staged, &theirs]);
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
    a_private_channel_reads_only_for_a_good_grant(rig, shape);
}

/// A read of a private channel, an observe, a fetch, a `newest` and a `list`, needs a grant that
/// is for this repository, covers what is read and has not expired, whatever its phase: a stage
/// grant and a commit grant that are bad are no better than a read grant that is.
fn a_private_channel_reads_only_for_a_good_grant<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    let (a, b) = (Set::of(rig, &["a"]), Set::of(rig, &["b"]));
    publish(rig, &mut p, &a);
    let k = &a.coords[0];
    let scope = ListScope {
        channel: k.channel.clone(),
        prefix: String::new(),
    };
    let phases = [GrantPhase::Read, GrantPhase::Stage, GrantPhase::Commit];
    for phase in phases {
        let foreign = rig.foreign_grant(&p, phase, &a.coords);
        let uncovered = rig.grant(&p, phase, &b.coords);
        for (what, grant) in [
            ("for another repository", &foreign),
            ("that does not name the coordinate", &uncovered),
        ] {
            is_denied(
                p.observe(grant, k, &[]),
                &format!("an observe with a {phase:?} grant {what} ({shape:?})"),
            );
            is_denied(
                p.fetch(grant, k),
                &format!("a fetch with a {phase:?} grant {what} ({shape:?})"),
            );
        }
        is_denied(
            p.newest(&foreign, &k.channel, &k.name),
            &format!("a newest with a {phase:?} grant for another repository ({shape:?})"),
        );
        is_denied(
            p.list(&foreign, &scope),
            &format!("a list with a {phase:?} grant for another repository ({shape:?})"),
        );
    }
    let old = phases.map(|phase| rig.grant(&p, phase, &a.coords));
    rig.advance(A_DAY_MS);
    for (phase, old) in phases.into_iter().zip(&old) {
        let refusals = [
            ("observe", p.observe(old, k, &[]).map(|_| ())),
            ("fetch", p.fetch(old, k).map(|_| ())),
            ("newest", p.newest(old, &k.channel, &k.name).map(|_| ())),
            ("list", p.list(old, &scope).map(|_| ())),
        ];
        for (call, answer) in refusals {
            same(
                answer,
                Err(SeamError::GrantRefused(GrantFault::Expired)),
                &format!("a {call} with an expired {phase:?} grant ({shape:?})"),
            );
        }
    }
}

/// Every read of a private channel needs a grant, a stranger's included, and a stranger is
/// `Denied` however the read is asked, an observe told the declared set included, and whatever
/// the coordinate is: one committed, one only staged or one another writer committed.
fn a_stranger_reads_nothing_of_a_private_channel<P: Publisher>(p: &mut P, sets: &[&Set]) {
    let none = Grant::none();
    for set in sets {
        let held = &set.coords[0];
        let blobs = set.blobs();
        let items = set.items(&blobs);
        let scope = ListScope {
            channel: held.channel.clone(),
            prefix: String::new(),
        };
        is_denied(
            p.observe(&none, held, &[]),
            "an observe by a stranger of a private channel",
        );
        is_denied(
            p.observe(&none, held, &items),
            "an observe told the declared set by a stranger of a private channel",
        );
        is_denied(
            p.fetch(&none, held),
            "a fetch by a stranger of a private channel",
        );
        is_denied(
            p.newest(&none, &held.channel, &held.name),
            "a newest by a stranger of a private channel",
        );
        is_denied(
            p.list(&none, &scope),
            "a list by a stranger of a private channel",
        );
    }
}
