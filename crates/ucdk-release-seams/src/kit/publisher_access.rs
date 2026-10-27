// SPDX-License-Identifier: Apache-2.0
//! The scenarios of the publisher kit about who may read and write a channel, how a private
//! channel is read, and what a call that went wrong left behind.

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::{Coordinate, Stamped};

use super::publisher::{
    A_DAY_MS, Arrangement, ChannelShape, PublisherRig, ReadStep, Set, fetched, observed,
    present_held, publish, sha, stage, staged_held, stranger,
};
use super::{coordinate, must, must_err, same};
use crate::{CommitAnswer, Grant, ListScope, Publisher, SeamError, StageAnswer};

fn denied(error: &SeamError) -> bool {
    *error == SeamError::GrantRefused(GrantFault::Denied)
}

fn held_by_another(set: &Set, at: usize, bytes: &[u8]) -> Arrangement {
    Arrangement::HeldByAnother {
        coordinate: set.coords[at].clone(),
        bytes: bytes.to_vec(),
    }
}

pub(super) fn public_read_needs_no_grant<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    publish(rig, &mut p, &set);
    // Staged state is nothing a stranger can read, however the read is asked.
    let only_staged = Set::of(&["b"]);
    stage(rig, &mut p, &only_staged);
    let b = &only_staged.coords[0];
    same(
        must(
            p.fetch(&Grant::none(), b),
            "fetch of a staged coordinate by a stranger",
        ),
        None,
        "the bytes of a staged coordinate fetched by a stranger",
    );
    same(
        must(
            p.newest(&Grant::none(), &b.channel, &b.name),
            "newest of a staged coordinate by a stranger",
        )
        .is_empty(),
        true,
        "a staged coordinate found with newest by a stranger",
    );
    let a = &set.coords[0];
    present_held(stranger(&mut p, a));
    same(
        must(p.fetch(&Grant::none(), a), "fetch by a stranger"),
        Some(b"a".to_vec()),
        "the bytes a stranger fetches",
    );
    let newest = must(
        p.newest(&Grant::none(), &a.channel, &a.name),
        "newest by a stranger",
    );
    same(
        newest.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>(),
        vec![a.clone()],
        "what a stranger finds with newest",
    );
    let scope = ListScope {
        channel: a.channel.clone(),
        prefix: String::new(),
    };
    let listed = must(p.list(&Grant::none(), &scope), "list by a stranger");
    same(
        listed.iter().all(|l| l.coordinate == *a),
        true,
        "what a stranger lists",
    );
}

/// A channel in which each write has something to change: a committed coordinate to yank, two
/// staged ones (one to commit, one to abort) and one nobody has written.
struct Writes {
    held: Set,
    to_commit: Set,
    to_abort: Set,
    fresh: Set,
    /// Every coordinate above, for a grant that names them all.
    all: Set,
}

impl Writes {
    /// Publishes `held` and stages `to_commit` and `to_abort` in `p`.
    fn arranged<R: PublisherRig>(rig: &R, p: &mut R::Publisher) -> Self {
        let writes = Self {
            held: Set::of(&["h"]),
            to_commit: Set::of(&["c"]),
            to_abort: Set::of(&["x"]),
            fresh: Set::of(&["n"]),
            all: Set::of(&["h", "c", "x", "n"]),
        };
        publish(rig, p, &writes.held);
        stage(rig, p, &writes.to_commit);
        stage(rig, p, &writes.to_abort);
        writes
    }
}

/// Holds that a stage, a commit, an abort and a yank with a bad grant (`stage_grant` for the
/// first and the third, `commit_grant` for the others) are each refused with `fault`, and that
/// what each tried to change is as it was.
fn every_write_refuses_a_bad_grant<R: PublisherRig>(
    rig: &R,
    p: &mut R::Publisher,
    writes: &Writes,
    (stage_grant, commit_grant): (&Grant, &Grant),
    fault: &SeamError,
    what: &str,
) {
    let blobs = writes.fresh.blobs();
    let items = writes.fresh.items(&blobs);
    same(
        must_err(p.stage(stage_grant, &items), "a stage with a bad grant"),
        fault.clone(),
        &format!("the answer to a stage with a grant {what}"),
    );
    same(
        observed(rig, p, GrantPhase::Stage, &writes.fresh, 0),
        Presence::Absent,
        &format!("the coordinate after a stage with a grant {what}"),
    );
    let blobs = writes.to_commit.blobs();
    let items = writes.to_commit.items(&blobs);
    same(
        must_err(p.commit(commit_grant, &items), "a commit with a bad grant"),
        fault.clone(),
        &format!("the answer to a commit with a grant {what}"),
    );
    staged_held(observed(rig, p, GrantPhase::Stage, &writes.to_commit, 0));
    same(
        fetched(rig, p, &writes.to_commit.coords[0]),
        None,
        &format!("the bytes after a commit with a grant {what}"),
    );
    same(
        must_err(
            p.abort(stage_grant, &writes.to_abort.coords),
            "an abort with a bad grant",
        ),
        fault.clone(),
        &format!("the answer to an abort with a grant {what}"),
    );
    staged_held(observed(rig, p, GrantPhase::Stage, &writes.to_abort, 0));
    same(
        must_err(
            p.yank(commit_grant, &writes.held.coords[0], "a reason"),
            "a yank with a bad grant",
        ),
        fault.clone(),
        &format!("the answer to a yank with a grant {what}"),
    );
    present_held(stranger(p, &writes.held.coords[0]));
}

pub(super) fn a_grant_for_another_repository_sends_nothing<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let writes = Writes::arranged(rig, &mut p);
    let stage_grant = rig.foreign_grant(&p, GrantPhase::Stage, &writes.all.coords);
    let commit_grant = rig.foreign_grant(&p, GrantPhase::Commit, &writes.all.coords);
    every_write_refuses_a_bad_grant(
        rig,
        &mut p,
        &writes,
        (&stage_grant, &commit_grant),
        &SeamError::GrantRefused(GrantFault::Denied),
        "for another repository",
    );
}

pub(super) fn a_grant_of_the_wrong_phase_is_denied_and_nothing_happens<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let commit_grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    let refusal = must_err(
        p.stage(&commit_grant, &items),
        "a stage with a commit grant",
    );
    same(
        denied(&refusal),
        true,
        "a stage with a commit grant is Denied",
    );
    same(
        observed(rig, &mut p, GrantPhase::Stage, &set, 0),
        Presence::Absent,
        "the coordinate after the refused stage",
    );
    stage(rig, &mut p, &set);
    let stage_grant = rig.grant(&p, GrantPhase::Stage, &set.coords);
    let refusal = must_err(
        p.commit(&stage_grant, &items),
        "a commit with a stage grant",
    );
    same(
        denied(&refusal),
        true,
        "a commit with a stage grant is Denied",
    );
    same(
        fetched(rig, &mut p, &set.coords[0]),
        None,
        "the bytes of the coordinate after the refused commit",
    );
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    a_grant_covers_the_coordinates_it_names_and_no_others(rig);
    a_yank_needs_the_commit_grant_that_names_it(rig);
    for shape in [ChannelShape::staging(), ChannelShape::private_grouped()] {
        a_stranger_writes_nothing(rig, &shape);
    }
}

/// A call with no grant is of no phase: it neither stages, commits, aborts nor yanks, and what
/// it tried to change stays as it was.
fn a_stranger_writes_nothing<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = rig.publisher(shape);
    let (held, staged, new) = (Set::of(&["h"]), Set::of(&["s"]), Set::of(&["n"]));
    publish(rig, &mut p, &held);
    stage(rig, &mut p, &staged);
    let none = Grant::none();
    let blobs = new.blobs();
    let items = new.items(&blobs);
    let refusals = [
        ("stage", must_err(p.stage(&none, &items), "stage")),
        ("commit", must_err(p.commit(&none, &items), "commit")),
        (
            "abort",
            must_err(p.abort(&none, &staged.coords), "abort of staged state"),
        ),
        (
            "yank",
            must_err(p.yank(&none, &held.coords[0], "a reason"), "yank"),
        ),
    ];
    for (call, refusal) in &refusals {
        same(
            denied(refusal),
            true,
            &format!("a {call} with no grant is Denied ({shape:?})"),
        );
    }
    same(
        observed(rig, &mut p, GrantPhase::Stage, &new, 0),
        Presence::Absent,
        "the coordinate a stranger tried to stage or commit",
    );
    same(fetched(rig, &mut p, &new.coords[0]), None, "its bytes");
    if !shape.group_addressed {
        // Staged state addressed by group is found only by an observe told the set: the
        // scenarios of that rule hold it, and this one does not depend on them.
        staged_held(observed(rig, &mut p, GrantPhase::Stage, &staged, 0));
    }
    present_held(stranger_or_read(rig, &mut p, &held.coords[0]));
}

/// What a read grant sees of `k`: on a private channel a stranger sees nothing.
fn stranger_or_read<R: PublisherRig>(rig: &R, p: &mut R::Publisher, k: &Coordinate) -> Presence {
    let grant = rig.grant(p, GrantPhase::Read, std::slice::from_ref(k));
    must(p.observe(&grant, k, &[]), "an observe with a read grant")
}

/// A grant of the right phase that names one of three coordinates does not stage, commit or
/// abort the three, whichever of them it names (the first, the last or the one between).
fn a_grant_covers_the_coordinates_it_names_and_no_others<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let trio = Set::of(&["c", "d", "e"]);
    let blobs = trio.blobs();
    let items = trio.items(&blobs);
    for one in 0..3 {
        let only = rig.grant(&p, GrantPhase::Stage, &trio.coords[one..=one]);
        let refusal = must_err(
            p.stage(&only, &items),
            "a stage of three coordinates with a grant for one",
        );
        same(
            denied(&refusal),
            true,
            &format!("a stage grant that names only coordinate {one} of three is Denied"),
        );
    }
    for at in 0..3 {
        same(
            observed(rig, &mut p, GrantPhase::Stage, &trio, at),
            Presence::Absent,
            "a coordinate after the refused stages",
        );
    }
    stage(rig, &mut p, &trio);
    for one in 0..3 {
        let only = rig.grant(&p, GrantPhase::Commit, &trio.coords[one..=one]);
        let refusal = must_err(
            p.commit(&only, &items),
            "a commit of three coordinates with a grant for one",
        );
        same(
            denied(&refusal),
            true,
            &format!("a commit grant that names only coordinate {one} of three is Denied"),
        );
    }
    for coord in &trio.coords {
        same(
            fetched(rig, &mut p, coord),
            None,
            "the bytes of a coordinate after the refused commits",
        );
    }
    for one in 0..3 {
        let only = rig.grant(&p, GrantPhase::Stage, &trio.coords[one..=one]);
        let refusal = must_err(
            p.abort(&only, &trio.coords),
            "an abort of three coordinates with a grant for one",
        );
        same(
            denied(&refusal),
            true,
            &format!("a stage grant that names only coordinate {one} of three cannot abort them"),
        );
    }
    for at in 0..3 {
        staged_held(observed(rig, &mut p, GrantPhase::Stage, &trio, at));
    }
}

/// Only a commit grant that names the coordinate yanks it, and what a refused yank tried to
/// yank stays as it was.
fn a_yank_needs_the_commit_grant_that_names_it<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let (set, other) = (Set::of(&["a"]), Set::of(&["b"]));
    publish(rig, &mut p, &set);
    let stage_grant = rig.grant(&p, GrantPhase::Stage, &set.coords);
    let refusal = must_err(
        p.yank(&stage_grant, &set.coords[0], "a reason"),
        "a yank with a stage grant",
    );
    same(
        denied(&refusal),
        true,
        "a yank with a stage grant is Denied",
    );
    let elsewhere = rig.grant(&p, GrantPhase::Commit, &other.coords);
    let refusal = must_err(
        p.yank(&elsewhere, &set.coords[0], "a reason"),
        "a yank with a commit grant that does not name the coordinate",
    );
    same(
        denied(&refusal),
        true,
        "a yank with a commit grant for another coordinate is Denied",
    );
    present_held(stranger(&mut p, &set.coords[0]));
}

pub(super) fn an_expired_grant_is_refused_and_nothing_happens<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let writes = Writes::arranged(rig, &mut p);
    let stage_grant = rig.grant(&p, GrantPhase::Stage, &writes.all.coords);
    let commit_grant = rig.grant(&p, GrantPhase::Commit, &writes.all.coords);
    rig.advance(A_DAY_MS);
    every_write_refuses_a_bad_grant(
        rig,
        &mut p,
        &writes,
        (&stage_grant, &commit_grant),
        &SeamError::GrantRefused(GrantFault::Expired),
        "that has expired",
    );
}

pub(super) fn newest_lists_every_held_coordinate_of_the_name<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let a1 = coordinate("a");
    let a2 = Coordinate {
        version: "1.2.4".to_owned(),
        ..a1.clone()
    };
    let a3 = Coordinate {
        platform: "darwin-arm64".to_owned(),
        ..a1.clone()
    };
    // Other names, one that starts with the name asked for, and the name in another channel.
    let elsewhere = Coordinate {
        channel: "beta".to_owned(),
        ..a1.clone()
    };
    let mut set = Set::at(vec![
        a1.clone(),
        a2.clone(),
        a3.clone(),
        coordinate("b"),
        coordinate("ab"),
        elsewhere,
    ]);
    set.held.exceptional = true;
    publish(rig, &mut p, &set);
    let grant = rig.grant(&p, GrantPhase::Commit, std::slice::from_ref(&a2));
    must(p.yank(&grant, &a2, "a reason"), "yank");
    let read = rig.grant(&p, GrantPhase::Read, std::slice::from_ref(&a1));
    let found = must(p.newest(&read, &a1.channel, &a1.name), "newest");
    for (c, held) in &found {
        same(
            held.clone(),
            Held {
                digest: sha(c.name.as_bytes()),
                ..set.held.clone()
            },
            &format!("what newest holds beside {c:?}: its digest, stamp and exceptional mark"),
        );
    }
    let key = |c: &Coordinate| (c.version.clone(), c.platform.clone());
    let mut got: Vec<Coordinate> = found.into_iter().map(|(c, _)| c).collect();
    got.sort_by_key(key);
    let mut want = vec![a1, a2, a3];
    want.sort_by_key(key);
    same(
        got,
        want,
        "every coordinate of the name in the channel, yanked ones included, and no other",
    );
}

pub(super) fn observe_returns_the_stamp_it_was_given<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let mut set = Set::of(&["a"]);
    set.held.stamp = Some(Stamped {
        build_number: 7,
        source_commit: "deadbeef".to_owned(),
    });
    set.held.exceptional = true;
    stage(rig, &mut p, &set);
    let staged = staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    same(
        (staged.stamp, staged.exceptional),
        (set.held.stamp.clone(), true),
        "the stamp and the exceptional mark of a staged coordinate",
    );
    super::publisher::commit(rig, &mut p, &set);
    let present = present_held(stranger(&mut p, &set.coords[0]));
    same(
        (present.stamp, present.exceptional),
        (set.held.stamp.clone(), true),
        "the stamp and the exceptional mark of a committed coordinate",
    );
}

pub(super) fn a_granted_observe_sees_a_coordinate_committed_by_another_writer<R: PublisherRig>(
    rig: &R,
) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    rig.arrange(&mut p, &held_by_another(&set, 0, b"theirs"));
    present_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
}

pub(super) fn a_private_observe_reads_the_group_then_the_index<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::private_grouped());
    let set = Set::of(&["a"]);
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

pub(super) fn a_coordinate_committed_in_another_group_reads_as_present_never_absent<
    R: PublisherRig,
>(
    rig: &R,
) {
    let mut p = rig.publisher(&ChannelShape::private_grouped());
    let set = Set::of(&["a"]);
    rig.arrange(&mut p, &held_by_another(&set, 0, b"theirs"));
    present_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
}

pub(super) fn a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent<
    R: PublisherRig,
>(
    rig: &R,
) {
    for (which, failure) in [
        ("group read", Arrangement::FailGroupRead),
        ("index read", Arrangement::FailIndexRead),
    ] {
        let mut p = rig.publisher(&ChannelShape::private_grouped());
        rig.arrange(&mut p, &failure);
        let set = Set::of(&["a"]);
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

pub(super) fn observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set<
    R: PublisherRig,
>(
    rig: &R,
) {
    let mut p = rig.publisher(&ChannelShape::private_grouped());
    let set = Set::of(&["a", "b"]);
    stage(rig, &mut p, &set);
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    // A set that does not hold the coordinate observed says nothing about it: Refused.
    let outside = Set::of(&["a", "b", "c"]);
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

pub(super) fn observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates<
    R: PublisherRig,
>(
    rig: &R,
) {
    let mut p = rig.publisher(&ChannelShape::private_grouped());
    let both = Set::of(&["a", "b"]);
    stage(rig, &mut p, &Set::of(&["a"]));
    rig.arrange(&mut p, &held_by_another(&both, 1, b"theirs"));
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

pub(super) fn a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit<
    R: PublisherRig,
>(
    rig: &R,
) {
    let mut p = rig.publisher(&ChannelShape {
        private: true,
        ..ChannelShape::staging()
    });
    let (a, b) = (Set::of(&["a"]), Set::of(&["b"]));
    publish(rig, &mut p, &a);
    a_stranger_reads_nothing_of_a_private_channel(&mut p, &a);
    let read_a = rig.grant(&p, GrantPhase::Read, &a.coords);
    let read_b = rig.grant(&p, GrantPhase::Read, &b.coords);
    present_held(must(
        p.observe(&read_a, &a.coords[0], &[]),
        "an observe with a read grant",
    ));
    let blobs = b.blobs();
    let items = b.items(&blobs);
    let refusals = [
        ("stage", must_err(p.stage(&read_b, &items), "stage")),
        ("commit", must_err(p.commit(&read_b, &items), "commit")),
        ("abort", must_err(p.abort(&read_a, &a.coords), "abort")),
        (
            "yank",
            must_err(p.yank(&read_a, &a.coords[0], "a reason"), "yank"),
        ),
    ];
    for (call, refusal) in &refusals {
        same(
            denied(refusal),
            true,
            &format!("a {call} with a read grant is Denied"),
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
    a_private_channel_reads_only_for_a_good_read_grant(rig);
}

/// A read of a private channel needs a read grant that is for this repository, covers what is
/// read and has not expired; a stranger's is refused in the scenario above.
fn a_private_channel_reads_only_for_a_good_read_grant<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape {
        private: true,
        ..ChannelShape::staging()
    });
    let (a, b) = (Set::of(&["a"]), Set::of(&["b"]));
    publish(rig, &mut p, &a);
    let k = &a.coords[0];
    let scope = ListScope {
        channel: k.channel.clone(),
        prefix: String::new(),
    };
    let foreign = rig.foreign_grant(&p, GrantPhase::Read, &a.coords);
    let uncovered = rig.grant(&p, GrantPhase::Read, &b.coords);
    for (what, grant) in [
        ("for another repository", &foreign),
        ("that does not name the coordinate", &uncovered),
    ] {
        let refusals = [
            ("observe", p.observe(grant, k, &[]).map(|_| ())),
            ("fetch", p.fetch(grant, k).map(|_| ())),
        ];
        for (call, answer) in refusals {
            same(
                answer,
                Err(SeamError::GrantRefused(GrantFault::Denied)),
                &format!("a {call} with a read grant {what}"),
            );
        }
    }
    for (call, answer) in [
        (
            "newest",
            p.newest(&foreign, &k.channel, &k.name).map(|_| ()),
        ),
        ("list", p.list(&foreign, &scope).map(|_| ())),
    ] {
        same(
            answer,
            Err(SeamError::GrantRefused(GrantFault::Denied)),
            &format!("a {call} with a read grant for another repository"),
        );
    }
    let old = rig.grant(&p, GrantPhase::Read, &a.coords);
    rig.advance(A_DAY_MS);
    let refusals = [
        ("observe", p.observe(&old, k, &[]).map(|_| ())),
        ("fetch", p.fetch(&old, k).map(|_| ())),
        ("newest", p.newest(&old, &k.channel, &k.name).map(|_| ())),
        ("list", p.list(&old, &scope).map(|_| ())),
    ];
    for (call, answer) in refusals {
        same(
            answer,
            Err(SeamError::GrantRefused(GrantFault::Expired)),
            &format!("a {call} with an expired read grant"),
        );
    }
}

/// Every read of a private channel needs a grant, a stranger's included, and a stranger is
/// `Denied` however the read is asked, an observe told the declared set included.
fn a_stranger_reads_nothing_of_a_private_channel<P: Publisher>(p: &mut P, set: &Set) {
    let none = Grant::none();
    let held = &set.coords[0];
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let scope = ListScope {
        channel: held.channel.clone(),
        prefix: String::new(),
    };
    let refusals = [
        ("observe", p.observe(&none, held, &[]).map(|_| ())),
        (
            "observe told the declared set",
            p.observe(&none, held, &items).map(|_| ()),
        ),
        ("fetch", p.fetch(&none, held).map(|_| ())),
        (
            "newest",
            p.newest(&none, &held.channel, &held.name).map(|_| ()),
        ),
        ("list", p.list(&none, &scope).map(|_| ())),
    ];
    for (call, answer) in refusals {
        same(
            answer,
            Err(SeamError::GrantRefused(GrantFault::Denied)),
            &format!("a {call} by a stranger of a private channel"),
        );
    }
}

pub(super) fn commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit<
    R: PublisherRig,
>(
    rig: &R,
) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    stage(rig, &mut p, &set);
    rig.arrange(&mut p, &Arrangement::RaceNextCommit(b"theirs".to_vec()));
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    same(
        must(p.commit(&grant, &items), "commit"),
        CommitAnswer::Duplicate,
        "a commit that lost the race",
    );
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(b"theirs".to_vec()),
        "the bytes after the race",
    );
}

pub(super) fn transient_after_commit_is_observed_before_retry<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    stage(rig, &mut p, &set);
    rig.arrange(&mut p, &Arrangement::FailAfterCommit);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    let failure = must_err(
        p.commit(&grant, &items),
        "a commit that fails after it landed",
    );
    same(
        failure.promises_no_effect(),
        false,
        "a failure after the commit may have landed",
    );
    present_held(stranger(&mut p, &set.coords[0]));
}

pub(super) fn duplicate_over_other_bytes_leaves_the_old_bytes<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let (a, b) = (Set::of(&["a"]), Set::of(&["b"]));
    rig.arrange(&mut p, &held_by_another(&a, 0, b"theirs"));
    rig.arrange(
        &mut p,
        &Arrangement::StagedByAnother {
            coordinate: b.coords[0].clone(),
            bytes: b"theirs".to_vec(),
        },
    );
    for set in [&a, &b] {
        let blobs = set.blobs();
        let items = set.items(&blobs);
        let grant = rig.grant(&p, GrantPhase::Stage, &set.coords);
        same(
            must(p.stage(&grant, &items), "stage"),
            StageAnswer::Duplicate,
            "a stage over the bytes of another writer",
        );
    }
    same(
        fetched(rig, &mut p, &a.coords[0]),
        Some(b"theirs".to_vec()),
        "the committed bytes after it",
    );
    same(
        observed(rig, &mut p, GrantPhase::Stage, &b, 0),
        Presence::Absent,
        "another writer's staged state seen by a grant that did not make it",
    );
    same(
        fetched(rig, &mut p, &b.coords[0]),
        None,
        "a coordinate only staged by another writer",
    );
}
