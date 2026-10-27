// SPDX-License-Identifier: Apache-2.0
//! The scenarios of the publisher kit about who may read and write a channel and what a call that
//! went wrong left behind: a stranger's reads of a public channel, the grants of another
//! repository, of the wrong phase, expired or not covering, what `newest` and `observe` report
//! beside a coordinate, and the commits that lose a race or fail after they landed.

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::{Coordinate, Stamped};

use super::publisher_grants::{self as grants, Spoiled, denied, is_denied, seen};
use super::publisher_rig::{
    A_DAY_MS, Arrangement, ChannelShape, Placement, PublisherRig, Set, built, commit, fetched,
    first_phase, held_by_another, observed, present_held, publish, sha, siblings_beside, stage,
    staged_held, staging_publisher, staging_shape, stranger,
};
use super::{must, must_err, same};
use crate::{CommitAnswer, Grant, ListScope, Publisher, SeamError, StageAnswer};

/// A read of a public channel needs no grant, and staged state is nothing a stranger can read,
/// however the read is asked: by `observe`, `fetch`, `newest` or `list`.
pub(super) fn public_read_needs_no_grant<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    publish(rig, &mut p, &set);
    let a = &set.coords[0];
    if p.stages() {
        let only_staged = Set::of(rig, &["b"]);
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
    }
    present_held(stranger(&mut p, a));
    same(
        must(p.fetch(&Grant::none(), a), "fetch by a stranger"),
        Some(set.bytes(0).to_vec()),
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
            held: Set::of(rig, &["h"]),
            to_commit: Set::of(rig, &["c"]),
            to_abort: Set::of(rig, &["x"]),
            fresh: Set::of(rig, &["n"]),
            all: Set::of(rig, &["h", "c", "x", "n"]),
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

/// A grant for another repository stages, commits, aborts and yanks nothing, on the channel that
/// stages and on every other shape the rig declares.
pub(super) fn a_grant_for_another_repository_sends_nothing<R: PublisherRig>(rig: &R) {
    if let Some(shape) = staging_shape(rig) {
        let mut p = built(rig, &shape);
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
    grants::every_shape_refuses_a_bad_write_grant(rig, Spoiled::Foreign);
}

/// A grant of the other write phase, a grant that names too little and a call with no grant at
/// all are each `Denied`, and what the call tried to change is as it was.
pub(super) fn a_grant_of_the_wrong_phase_is_denied_and_nothing_happens<R: PublisherRig>(rig: &R) {
    if staging_shape(rig).is_some() {
        a_commit_grant_stages_nothing_and_a_stage_grant_commits_nothing(rig);
        a_grant_covers_the_coordinates_it_names_and_no_others(rig);
        a_yank_needs_the_commit_grant_that_names_it(rig);
        grants::an_abort_needs_the_stage_grant_that_names_it(rig);
    }
    grants::a_grant_names_every_field_of_the_coordinates_it_covers(rig);
    grants::every_shape_refuses_a_bad_write_grant(rig, Spoiled::Uncovered);
    grants::every_shape_refuses_a_bad_write_grant(rig, Spoiled::OtherPhase);
    for shape in rig.shapes() {
        a_stranger_writes_nothing(rig, &shape);
    }
}

/// A commit grant does not stage, and a stage grant does not commit.
fn a_commit_grant_stages_nothing_and_a_stage_grant_commits_nothing<R: PublisherRig>(rig: &R) {
    let mut p = staging_publisher(rig);
    let set = Set::of(rig, &["a"]);
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
}

/// A call with no grant is of no phase: it neither stages, commits, aborts nor yanks, and what
/// it tried to change stays as it was. A channel that does not stage is not asked to stage or to
/// abort.
fn a_stranger_writes_nothing<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    let (held, staged, new) = (
        Set::of(rig, &["h"]),
        Set::of(rig, &["s"]),
        Set::of(rig, &["n"]),
    );
    publish(rig, &mut p, &held);
    stage(rig, &mut p, &staged);
    let none = Grant::none();
    let blobs = new.blobs();
    let items = new.items(&blobs);
    let mut refusals = vec![
        ("commit", must_err(p.commit(&none, &items), "commit")),
        (
            "yank",
            must_err(p.yank(&none, &held.coords[0], "a reason"), "yank"),
        ),
    ];
    if p.stages() {
        refusals.push(("stage", must_err(p.stage(&none, &items), "stage")));
        refusals.push((
            "abort",
            must_err(p.abort(&none, &staged.coords), "abort of staged state"),
        ));
    }
    for (call, refusal) in &refusals {
        same(
            denied(refusal),
            true,
            &format!("a {call} with no grant is Denied ({shape:?})"),
        );
    }
    let phase = first_phase(&p);
    same(
        observed(rig, &mut p, phase, &new, 0),
        Presence::Absent,
        "the coordinate a stranger tried to stage or commit",
    );
    same(fetched(rig, &mut p, &new.coords[0]), None, "its bytes");
    if p.stages() && !shape.group_addressed {
        // Staged state addressed by group is found only by an observe told the set: the
        // scenarios of that rule hold it, and this one does not depend on them.
        staged_held(observed(rig, &mut p, GrantPhase::Stage, &staged, 0));
    }
    present_held(seen(rig, &mut p, &held.coords[0]));
}

/// A grant of the right phase that names one of three coordinates does not stage, commit or
/// abort the three, whichever of them it names (the first, the last or the one between).
fn a_grant_covers_the_coordinates_it_names_and_no_others<R: PublisherRig>(rig: &R) {
    let mut p = staging_publisher(rig);
    let trio = Set::of(rig, &["c", "d", "e"]);
    let blobs = trio.blobs();
    let items = trio.items(&blobs);
    for one in 0..3 {
        let only = rig.grant(&p, GrantPhase::Stage, &trio.coords[one..=one]);
        is_denied(
            p.stage(&only, &items),
            &format!("a stage grant that names only coordinate {one} of three"),
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
        is_denied(
            p.commit(&only, &items),
            &format!("a commit grant that names only coordinate {one} of three"),
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
        is_denied(
            p.abort(&only, &trio.coords),
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
    let mut p = staging_publisher(rig);
    let (set, other) = (Set::of(rig, &["a"]), Set::of(rig, &["b"]));
    publish(rig, &mut p, &set);
    let stage_grant = rig.grant(&p, GrantPhase::Stage, &set.coords);
    is_denied(
        p.yank(&stage_grant, &set.coords[0], "a reason"),
        "a yank with a stage grant",
    );
    let elsewhere = rig.grant(&p, GrantPhase::Commit, &other.coords);
    is_denied(
        p.yank(&elsewhere, &set.coords[0], "a reason"),
        "a yank with a commit grant that does not name the coordinate",
    );
    present_held(stranger(&mut p, &set.coords[0]));
}

/// An expired grant stages, commits, aborts and yanks nothing and reads nothing it was not good
/// for, on the channel that stages and on every other shape the rig declares.
pub(super) fn an_expired_grant_is_refused_and_nothing_happens<R: PublisherRig>(rig: &R) {
    if let Some(shape) = staging_shape(rig) {
        let mut p = built(rig, &shape);
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
    grants::every_shape_refuses_a_bad_write_grant(rig, Spoiled::Expired);
}

/// `newest` answers every held coordinate of the name in the channel, of any version and
/// platform and yanked ones included, each with the digest, stamp and exceptional mark it holds,
/// and no coordinate of another name, of a name that extends it, or of another channel.
pub(super) fn newest_lists_every_held_coordinate_of_the_name<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let a1 = rig.coordinate("a");
    // The coordinates of the name, which a call to newest must find, and the others, which it
    // must not: other names, one that starts with the name asked for, and the name in another
    // channel.
    let mut together = vec![a1.clone()];
    let mut alone = Vec::new();
    let mut want = vec![a1.clone()];
    let mut other = vec![rig.coordinate("b"), rig.coordinate("ab")];
    for sibling in siblings_beside(rig, &a1) {
        match (sibling.field, sibling.placement) {
            ("channel", _) => alone.push(sibling.coordinate),
            ("version" | "platform", Placement::SameSet) => {
                together.push(sibling.coordinate.clone());
                want.push(sibling.coordinate);
            }
            ("version" | "platform", Placement::OwnSet) => {
                alone.push(sibling.coordinate.clone());
                want.push(sibling.coordinate);
            }
            _ => {}
        }
    }
    together.append(&mut other);
    let mut sets = vec![Set::at(rig, together)];
    sets.extend(alone.into_iter().map(|c| Set::at(rig, vec![c])));
    for set in &mut sets {
        set.held.exceptional = true;
        publish(rig, &mut p, set);
    }
    let template = sets[0].held.clone();
    let target = want[want.len() - 1].clone();
    let grant = rig.grant(&p, GrantPhase::Commit, &want);
    must(p.yank(&grant, &target, "a reason"), "yank");
    let read = rig.grant(&p, GrantPhase::Read, std::slice::from_ref(&a1));
    let found = must(p.newest(&read, &a1.channel, &a1.name), "newest");
    for (c, held) in &found {
        same(
            held.clone(),
            Held {
                digest: sha(&rig.payload(c, 0).bytes),
                ..template.clone()
            },
            &format!("what newest holds beside {c:?}: its digest, stamp and exceptional mark"),
        );
    }
    let key = |c: &Coordinate| (c.version.clone(), c.platform.clone());
    let mut got: Vec<Coordinate> = found.into_iter().map(|(c, _)| c).collect();
    got.sort_by_key(key);
    want.sort_by_key(key);
    same(
        got,
        want,
        "every coordinate of the name in the channel, yanked ones included, and no other",
    );
}

/// An observe carries the stamp and the exceptional mark a coordinate was written with, staged
/// and committed.
pub(super) fn observe_returns_the_stamp_it_was_given<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let mut set = Set::of(rig, &["a"]);
    set.held.stamp = Some(Stamped {
        build_number: 7,
        source_commit: "deadbeef".to_owned(),
    });
    set.held.exceptional = true;
    stage(rig, &mut p, &set);
    if p.stages() {
        let staged = staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
        same(
            (staged.stamp, staged.exceptional),
            (set.held.stamp.clone(), true),
            "the stamp and the exceptional mark of a staged coordinate",
        );
    }
    commit(rig, &mut p, &set);
    let present = present_held(stranger(&mut p, &set.coords[0]));
    same(
        (present.stamp, present.exceptional),
        (set.held.stamp.clone(), true),
        "the stamp and the exceptional mark of a committed coordinate",
    );
}

/// A grant that observes a coordinate another writer committed sees it as `Present`.
pub(super) fn a_granted_observe_sees_a_coordinate_committed_by_another_writer<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    rig.arrange(&mut p, &held_by_another(rig, &set.coords[0]));
    let phase = first_phase(&p);
    present_held(observed(rig, &mut p, phase, &set, 0));
}

/// A commit that loses a race to another writer's commit of the version answers `Duplicate` and
/// leaves the bytes that won.
pub(super) fn commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit<
    R: PublisherRig,
>(
    rig: &R,
) {
    let mut p = staging_publisher(rig);
    let set = Set::of(rig, &["a"]);
    let theirs = rig.payload(&set.coords[0], 1).bytes;
    stage(rig, &mut p, &set);
    rig.arrange(&mut p, &Arrangement::RaceNextCommit(theirs.clone()));
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
        Some(theirs),
        "the bytes after the race",
    );
}

/// A commit that fails after it landed is not `Permanent`: it may have landed, and an observe
/// finds out.
pub(super) fn transient_after_commit_is_observed_before_retry<R: PublisherRig>(rig: &R) {
    let mut p = staging_publisher(rig);
    let set = Set::of(rig, &["a"]);
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

/// A stage over a coordinate another writer holds or has staged is `Duplicate` and leaves what
/// is there: the committed bytes stay, and the staged state of another writer is no more visible
/// to a grant that did not make it than before.
pub(super) fn duplicate_over_other_bytes_leaves_the_old_bytes<R: PublisherRig>(rig: &R) {
    let mut p = staging_publisher(rig);
    let (a, b) = (Set::of(rig, &["a"]), Set::of(rig, &["b"]));
    rig.arrange(&mut p, &held_by_another(rig, &a.coords[0]));
    rig.arrange(
        &mut p,
        &Arrangement::StagedByAnother {
            coordinate: b.coords[0].clone(),
            bytes: rig.payload(&b.coords[0], 1).bytes,
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
        Some(rig.payload(&a.coords[0], 1).bytes),
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
