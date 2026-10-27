// SPDX-License-Identifier: Apache-2.0
//! Scenarios of the publisher kit about what a call that was refused, or answered `Duplicate`,
//! leaves behind; about a commit and what was staged; about sets; and about coordinates of one
//! name that differ in a field.

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::Presence;
use ucdk_release_proto::v1::{Coordinate, Stamped};

use super::publisher_rig::{
    Arrangement, ChannelShape, Placement, PublisherRig, Set, built, commit, fetched,
    held_by_another, holder_of, is_yanked, looked, observed, present_held, publish, sets_beside,
    siblings_beside, stage, staged_held, staging_shapes,
};
use super::publisher_visible::found_nothing;
use super::{must, must_err, same};
use crate::{CommitAnswer, CommitUnit, ListScope, Publisher, SeamError, StageAnswer, YankUnit};

/// A commit carries what was staged: other bytes, another stamp, or a coordinate nobody staged
/// on a channel that stages are each refused, and the staged state, which a commit of what was
/// staged still makes readable, is as it was.
pub(super) fn a_commit_carries_exactly_what_was_staged<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    stage(rig, &mut p, &set);
    let other_bytes = set.reseeded(rig, 1);
    let mut other_stamp = set.clone();
    other_stamp.held.stamp = Some(Stamped {
        build_number: 42,
        source_commit: "def456".to_owned(),
    });
    for (what, wrong) in [
        ("other bytes", &other_bytes),
        ("another stamp", &other_stamp),
    ] {
        let blobs = wrong.blobs();
        let items = wrong.items(&blobs);
        let grant = rig.grant(&p, GrantPhase::Commit, &wrong.coords);
        must_err(
            p.commit(&grant, &items),
            &format!("a commit of {what} over what was staged"),
        );
        same(
            fetched(rig, &mut p, &set.coords[0]),
            None,
            &format!("the bytes after a commit of {what}"),
        );
        staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    }
    let unstaged = Set::of(rig, &["u"]);
    let blobs = unstaged.blobs();
    let items = unstaged.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &unstaged.coords);
    must_err(
        p.commit(&grant, &items),
        "a commit of a coordinate nobody staged, on a channel that stages",
    );
    same(
        fetched(rig, &mut p, &unstaged.coords[0]),
        None,
        "the bytes of a coordinate nobody staged after a commit of it",
    );
    same(
        observed(rig, &mut p, GrantPhase::Stage, &unstaged, 0),
        Presence::Absent,
        "a coordinate nobody staged after a commit of it",
    );
    commit(rig, &mut p, &set);
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(set.bytes(0).to_vec()),
        "the bytes of what was staged, committed after the refusals",
    );
    a_commit_publishes_the_coordinate_it_names_and_not_another_that_is_staged(rig, shape);
}

/// With two coordinates staged, a commit that names one publishes the bytes that were staged at
/// it and nothing of the other, which stays staged and is committed by its own commit.
fn a_commit_publishes_the_coordinate_it_names_and_not_another_that_is_staged<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let (a, b) = (Set::of(rig, &["a"]), Set::of(rig, &["b"]));
    stage(rig, &mut p, &a);
    stage(rig, &mut p, &b);
    commit(rig, &mut p, &a);
    same(
        fetched(rig, &mut p, &a.coords[0]),
        Some(a.bytes(0).to_vec()),
        "the bytes of the coordinate a commit named, with another staged",
    );
    same(
        fetched(rig, &mut p, &b.coords[0]),
        None,
        "the bytes of a staged coordinate a commit did not name",
    );
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &b, 0));
    commit(rig, &mut p, &b);
    same(
        fetched(rig, &mut p, &b.coords[0]),
        Some(b.bytes(0).to_vec()),
        "the bytes of the coordinate committed second",
    );
    a_commit_publishes_the_coordinate_it_names_and_not_a_sibling_that_is_staged(rig, shape);
}

/// With a coordinate and a sibling of it that differs in a field staged, each in a set of its own,
/// a commit that names the base publishes the bytes staged at the base and leaves the sibling
/// staged.
fn a_commit_publishes_the_coordinate_it_names_and_not_a_sibling_that_is_staged<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let base = rig.coordinate("a");
    for sibling in siblings_beside(rig, &base) {
        if sibling.placement != Placement::OwnSet {
            continue;
        }
        let field = sibling.field;
        let mut p = built(rig, shape);
        let (named, other) = (
            Set::at(rig, vec![base.clone()]),
            Set::at(rig, vec![sibling.coordinate.clone()]),
        );
        stage(rig, &mut p, &named);
        stage(rig, &mut p, &other);
        commit(rig, &mut p, &named);
        same(
            fetched(rig, &mut p, &base),
            Some(named.bytes(0).to_vec()),
            &format!(
                "the bytes of a coordinate committed beside a staged one that differs in its {field}"
            ),
        );
        same(
            fetched(rig, &mut p, &sibling.coordinate),
            None,
            &format!(
                "the bytes of a staged coordinate that differs in its {field} from the one committed"
            ),
        );
        staged_held(observed(rig, &mut p, GrantPhase::Stage, &other, 0));
        commit(rig, &mut p, &other);
        same(
            fetched(rig, &mut p, &sibling.coordinate),
            Some(other.bytes(0).to_vec()),
            &format!("the bytes of the sibling that differs in its {field}, committed second"),
        );
    }
}

/// A commit of a set that is refused for one coordinate applies none of the others: a
/// coordinate nobody staged, on a channel that stages, is refused, and the coordinate beside it
/// is no more readable than before and is still staged. A channel that commits each coordinate
/// on its own refuses any call that names two, which the scenario of the set commit holds.
pub(super) fn a_set_commit_that_is_refused_applies_none_of_it<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    if !shape.stages {
        return;
    }
    let mut p = built(rig, shape);
    let b = Set::of(rig, &["b"]);
    stage(rig, &mut p, &b);
    let both = Set::of(rig, &["b", "u"]);
    let blobs = both.blobs();
    let items = both.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &both.coords);
    must_err(
        p.commit(&grant, &items),
        "a commit of a set one coordinate of which nobody staged",
    );
    same(
        fetched(rig, &mut p, &b.coords[0]),
        None,
        "the staged coordinate of a set whose commit was refused",
    );
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &b, 0));
}

/// A stage or a commit of a set one coordinate of which is held by another writer answers
/// `Duplicate` and applies none of the set: the others are not staged, not committed. Run on
/// every declared channel that commits a set at once.
pub(super) fn a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    if shape.stages {
        a_stage_of_a_set_that_meets_a_held_coordinate(rig, shape);
    }
    a_commit_of_a_set_that_meets_a_held_coordinate(rig, shape);
}

fn a_stage_of_a_set_that_meets_a_held_coordinate<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    rig.arrange(&mut p, &held_by_another(rig, &rig.coordinate("a")));
    let two = Set::of(rig, &["c", "a"]);
    let blobs = two.blobs();
    let items = two.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Stage, &two.coords);
    same(
        must(p.stage(&grant, &items), "a stage of a set"),
        StageAnswer::Duplicate,
        "a stage of a set one coordinate of which is held",
    );
    let free = Set::of(rig, &["c"]);
    same(
        observed(rig, &mut p, GrantPhase::Stage, &free, 0),
        Presence::Absent,
        "the free coordinate of a set whose stage was answered Duplicate",
    );
    the_held_coordinate_is_as_the_other_writer_left_it(
        rig,
        shape,
        &mut p,
        &two.coords[1],
        "a stage",
    );
}

/// The coordinate another writer holds, after a call that met it was answered `Duplicate`, still
/// reads as that writer's bytes and is not yanked.
fn the_held_coordinate_is_as_the_other_writer_left_it<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
    p: &mut R::Publisher,
    held: &Coordinate,
    after: &str,
) {
    same(
        fetched(rig, p, held),
        Some(set_of_another(rig, held)),
        &format!("the held coordinate after {after} of a set that met it"),
    );
    present_held(looked(rig, shape, p, held));
}

fn a_commit_of_a_set_that_meets_a_held_coordinate<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    rig.arrange(&mut p, &held_by_another(rig, &rig.coordinate("a")));
    let b = Set::of(rig, &["b"]);
    stage(rig, &mut p, &b);
    // The held coordinate is the last of the set: a registry that looks at the first alone is
    // held to the answer for the whole.
    let two = Set::of(rig, &["b", "a"]);
    let blobs = two.blobs();
    let items = two.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &two.coords);
    same(
        must(p.commit(&grant, &items), "a commit of a set"),
        CommitAnswer::Duplicate,
        &format!("a commit of a set one coordinate of which is held ({shape:?})"),
    );
    same(
        fetched(rig, &mut p, &b.coords[0]),
        None,
        &format!("the free coordinate of a set whose commit was Duplicate ({shape:?})"),
    );
    the_held_coordinate_is_as_the_other_writer_left_it(
        rig,
        shape,
        &mut p,
        &two.coords[1],
        "a commit",
    );
    if shape.stages {
        staged_held(observed(rig, &mut p, GrantPhase::Stage, &b, 0));
    }
}

/// The bytes another writer's member of `k` holds.
fn set_of_another<R: PublisherRig>(rig: &R, k: &Coordinate) -> Vec<u8> {
    rig.payload(k, 1).bytes
}

/// A set that names one coordinate twice could carry two byte strings for it: a stage (on a
/// channel that stages), a commit and an observe refuse it and change nothing. Run on every
/// declared channel: on one that does not stage, the commit carries both byte strings and is the
/// only call that can be refused.
pub(super) fn a_set_that_names_a_coordinate_twice_is_refused<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let a = rig.coordinate("a");
    let mut twice = Set::at(rig, vec![a.clone(), a.clone()]);
    twice.payloads[1] = rig.payload(&a, 1);
    let blobs = twice.blobs();
    let items = twice.items(&blobs);
    let once = Set::of(rig, &["a"]);
    if p.stages() {
        let stage_grant = rig.grant(&p, GrantPhase::Stage, std::slice::from_ref(&a));
        must_err(
            p.stage(&stage_grant, &items),
            "a stage that names a coordinate twice",
        );
        same(
            observed(rig, &mut p, GrantPhase::Stage, &once, 0),
            Presence::Absent,
            "the coordinate after a refused stage of a set that names it twice",
        );
        must_err(
            p.observe(&stage_grant, &a, &items),
            "an observe told a set that names a coordinate twice",
        );
    }
    // Staged with the bytes of the last of the two, so that a commit that keeps the last of them
    // would be one of what was staged.
    let last = once.reseeded(rig, 1);
    stage(rig, &mut p, &last);
    let commit_grant = rig.grant(&p, GrantPhase::Commit, std::slice::from_ref(&a));
    must_err(
        p.commit(&commit_grant, &items),
        "a commit that names a coordinate twice",
    );
    same(
        fetched(rig, &mut p, &a),
        None,
        "the bytes after a refused commit of a set that names a coordinate twice",
    );
    if p.stages() {
        staged_held(observed(rig, &mut p, GrantPhase::Stage, &last, 0));
    }
}

/// Staged state is no more readable to the one who staged it than to a stranger, but for an
/// observe, and an observe shows it only to a stage grant and a commit grant: a stage grant, a
/// commit grant and a read grant fetch nothing of it and find nothing of it with `newest` or
/// `list`, and a read grant observes it `Absent`, told the declared set or not. Held on every
/// channel that stages: a public one, a private one and one addressed by group. A stage grant and
/// a commit grant are for `observe` only, so a private channel may refuse their `fetch`, `newest`
/// and `list` with `Denied` as well as answer nothing; a read grant is for reading, and is never
/// refused.
pub(super) fn a_stage_or_commit_grant_reads_no_staged_state_but_through_observe<R: PublisherRig>(
    rig: &R,
) {
    for shape in staging_shapes(rig) {
        let mut p = built(rig, &shape);
        let set = Set::of(rig, &["a"]);
        stage(rig, &mut p, &set);
        let a = &set.coords[0];
        let scope = ListScope {
            channel: a.channel.clone(),
            prefix: String::new(),
        };
        for phase in [GrantPhase::Stage, GrantPhase::Commit, GrantPhase::Read] {
            let grant = rig.grant(&p, phase, &set.coords);
            let may_refuse = shape.private & (phase != GrantPhase::Read);
            found_nothing(
                p.fetch(&grant, a),
                Option::is_none,
                may_refuse,
                &format!(
                    "the bytes of a staged coordinate fetched with a {phase:?} grant ({shape:?})"
                ),
            );
            found_nothing(
                p.newest(&grant, &a.channel, &a.name),
                Vec::is_empty,
                may_refuse,
                &format!("a staged coordinate found with newest and a {phase:?} grant ({shape:?})"),
            );
            found_nothing(
                p.list(&grant, &scope),
                Vec::is_empty,
                may_refuse,
                &format!("a staged coordinate listed with a {phase:?} grant ({shape:?})"),
            );
        }
        let blobs = set.blobs();
        let items = set.items(&blobs);
        let read = rig.grant(&p, GrantPhase::Read, &set.coords);
        for (told, set_told) in [("no set", &[][..]), ("the declared set", &items[..])] {
            same(
                must(
                    p.observe(&read, a, set_told),
                    "an observe of a staged coordinate with a read grant",
                ),
                Presence::Absent,
                &format!("a staged coordinate observed with a read grant told {told} ({shape:?})"),
            );
        }
        staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    }
}

/// On a channel that does not stage, a coordinate is created once too: a commit that lost a
/// race leaves the bytes that won, a commit over a yanked coordinate is not a commit, a failure
/// after a commit that landed is not `Permanent`, and a set commit that fails makes nothing
/// readable.
pub(super) fn a_channel_that_does_not_stage_creates_a_coordinate_once<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let set = Set::of(rig, &["a"]);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let a = &set.coords[0];

    let mut p = built(rig, shape);
    rig.arrange(&mut p, &Arrangement::RaceNextCommit(set_of_another(rig, a)));
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    same(
        must(p.commit(&grant, &items), "a commit that lost a race"),
        CommitAnswer::Duplicate,
        "the answer to a commit that lost a race on a channel that does not stage",
    );
    same(
        fetched(rig, &mut p, a),
        Some(set_of_another(rig, a)),
        "the bytes after a race on a channel that does not stage",
    );

    let mut p = built(rig, shape);
    commit(rig, &mut p, &set);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    must(p.yank(&grant, a, "a reason"), "yank");
    let again = p.commit(&grant, &items);
    same(
        matches!(again, Ok(CommitAnswer::Committed)),
        false,
        "a commit over a yanked coordinate on a channel that does not stage",
    );
    same(
        is_yanked(&looked(rig, shape, &mut p, a)),
        true,
        "the coordinate after the second commit, on a channel that does not stage",
    );

    let mut p = built(rig, shape);
    rig.arrange(&mut p, &Arrangement::FailAfterCommit);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    let failure = must_err(
        p.commit(&grant, &items),
        "a commit that fails after it landed",
    );
    same(
        failure.promises_no_effect(),
        false,
        "a failure after the commit landed, on a channel that does not stage",
    );
    present_held(looked(rig, shape, &mut p, a));

    let mut p = built(rig, shape);
    if p.commit_unit() == CommitUnit::Set {
        let two = Set::of(rig, &["a", "b"]);
        rig.arrange(&mut p, &Arrangement::CommitSetFailsWhole);
        let blobs = two.blobs();
        let items = two.items(&blobs);
        let grant = rig.grant(&p, GrantPhase::Commit, &two.coords);
        must_err(p.commit(&grant, &items), "a set commit that fails");
        for coord in &two.coords {
            same(
                fetched(rig, &mut p, coord),
                None,
                "the bytes of a coordinate of a set that failed, on a channel that does not stage",
            );
        }
    }
}

/// A coordinate that differs from another of its name in one field is another coordinate: it is
/// staged and committed beside a held one, and an observe of it finds nothing of the staged
/// state of the other. A sibling the channel takes only with the base is published in one set
/// with it.
pub(super) fn a_sibling_is_another_coordinate_to_stage_commit_and_observe<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let base = rig.coordinate("a");
    for sibling in siblings_beside(rig, &base) {
        let field = sibling.field;
        let mut p = built(rig, shape);
        for set in sets_beside(rig, &base, &sibling) {
            publish(rig, &mut p, &set);
        }
        present_held(looked(rig, shape, &mut p, &base));
        present_held(looked(rig, shape, &mut p, &sibling.coordinate));
        same(
            fetched(rig, &mut p, &sibling.coordinate),
            Some(rig.payload(&sibling.coordinate, 0).bytes),
            &format!("the bytes of a coordinate beside one that differs in its {field}"),
        );

        let mut p = built(rig, shape);
        if p.stages() {
            stage(rig, &mut p, &Set::at(rig, vec![base.clone()]));
            let grant = rig.grant(
                &p,
                GrantPhase::Stage,
                std::slice::from_ref(&sibling.coordinate),
            );
            same(
                must(
                    p.observe(&grant, &sibling.coordinate, &[]),
                    "an observe of a sibling of a staged coordinate",
                ),
                Presence::Absent,
                &format!("a coordinate that differs in its {field} from one that is staged"),
            );
        }
    }
}

/// An abort gives back the staged state of the coordinates it names and of no sibling of theirs
/// that differs in a field; what it gave back can be staged again. Staged state is observed told
/// the set it was staged in, as a channel addressed by group finds it only so.
pub(super) fn an_abort_reaches_only_the_coordinates_it_names<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let base = rig.coordinate("a");
    for sibling in siblings_beside(rig, &base) {
        let mut p = built(rig, shape);
        let named = Set::at(rig, vec![base.clone()]);
        let sets = sets_beside(rig, &base, &sibling);
        for set in &sets {
            stage(rig, &mut p, set);
        }
        let grant = rig.grant(&p, GrantPhase::Stage, &named.coords);
        must(p.abort(&grant, &named.coords), "abort");
        let (staged_with, at) = holder_of(&sets, &base);
        same(
            observed(rig, &mut p, GrantPhase::Stage, staged_with, at),
            Presence::Absent,
            &format!("the coordinate an abort named ({shape:?})"),
        );
        let (staged_with, at) = holder_of(&sets, &sibling.coordinate);
        staged_held(observed(rig, &mut p, GrantPhase::Stage, staged_with, at));
        stage(rig, &mut p, &named);
    }
    an_abort_leaves_a_committed_coordinate_it_does_not_name_alone(rig, shape);
}

/// An abort of a staged coordinate does not drop a committed one it does not name.
fn an_abort_leaves_a_committed_coordinate_it_does_not_name_alone<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let (committed, staged) = (Set::of(rig, &["a"]), Set::of(rig, &["b"]));
    publish(rig, &mut p, &committed);
    stage(rig, &mut p, &staged);
    let grant = rig.grant(&p, GrantPhase::Stage, &staged.coords);
    must(p.abort(&grant, &staged.coords), "abort");
    present_held(looked(rig, shape, &mut p, &committed.coords[0]));
    same(
        fetched(rig, &mut p, &committed.coords[0]),
        Some(committed.bytes(0).to_vec()),
        "the bytes of a committed coordinate an abort did not name",
    );
    // A committed coordinate stays as it was when the abort names a sibling of it that differs in
    // a field.
    let base = rig.coordinate("a");
    for sibling in siblings_beside(rig, &base) {
        if sibling.placement != Placement::OwnSet {
            continue;
        }
        let mut p = built(rig, shape);
        let (held, named) = (
            Set::at(rig, vec![base.clone()]),
            Set::at(rig, vec![sibling.coordinate.clone()]),
        );
        publish(rig, &mut p, &held);
        stage(rig, &mut p, &named);
        let grant = rig.grant(&p, GrantPhase::Stage, &named.coords);
        must(p.abort(&grant, &named.coords), "abort");
        present_held(looked(rig, shape, &mut p, &base));
        same(
            fetched(rig, &mut p, &base),
            Some(held.bytes(0).to_vec()),
            &format!(
                "the bytes of a committed coordinate after an abort of one that differs in its {}",
                sibling.field
            ),
        );
    }
}

/// A yank yanks the coordinate it names and no sibling of it, unless the channel's `yank_unit()`
/// is the version, which is held both ways: a channel that yanks a version yanks every held
/// platform of it when the grant covers them all and refuses, changing nothing, when the grant
/// leaves one out; a channel that yanks a coordinate leaves its sibling alone. A coordinate
/// nobody holds is no coordinate to yank.
pub(super) fn a_yank_reaches_only_the_coordinate_it_names_and_needs_it_held<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let a = rig.coordinate("a");
    for sibling in siblings_beside(rig, &a) {
        let mut p = built(rig, shape);
        for set in sets_beside(rig, &a, &sibling) {
            publish(rig, &mut p, &set);
        }
        let of_the_version = sibling.field == "platform" && p.yank_unit() == YankUnit::Version;
        if of_the_version {
            yank_a_version(rig, shape, &mut p, &a, &sibling.coordinate);
        } else {
            let grant = rig.grant(&p, GrantPhase::Commit, std::slice::from_ref(&a));
            must(p.yank(&grant, &a, "a reason"), "yank");
            same(
                is_yanked(&looked(rig, shape, &mut p, &a)),
                true,
                &format!("the coordinate a yank named ({shape:?})"),
            );
            present_held(looked(rig, shape, &mut p, &sibling.coordinate));
        }
    }
    a_version_yank_reaches_no_other_version(rig, shape);
    a_yank_names_a_coordinate_and_not_what_was_committed_with_it_or_what_extends_its_name(
        rig, shape,
    );
    let mut p = built(rig, shape);
    let nothing = rig.coordinate("x");
    let grant = rig.grant(&p, GrantPhase::Commit, std::slice::from_ref(&nothing));
    must_err(
        p.yank(&grant, &nothing, "a reason"),
        "a yank of a coordinate nobody holds",
    );
    same(
        looked(rig, shape, &mut p, &nothing),
        Presence::Absent,
        "a coordinate nobody holds after a yank of it",
    );
}

/// A channel that yanks a version yanks the platforms of that version and of no other: with
/// another version of the name held beside them, a yank that names the coordinate and its platform
/// leaves the other version as it was.
fn a_version_yank_reaches_no_other_version<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let a = rig.coordinate("a");
    let siblings = siblings_beside(rig, &a);
    let of = |field: &str| siblings.iter().find(|s| s.field == field);
    let (Some(platform), Some(version)) = (of("platform"), of("version")) else {
        return;
    };
    let mut p = built(rig, shape);
    if p.yank_unit() != YankUnit::Version {
        return;
    }
    // The coordinates the channel takes only with the base are published with it, and the others
    // each in a set of their own.
    let mut together = vec![a.clone()];
    let mut alone = Vec::new();
    for sibling in [platform, version] {
        match sibling.placement {
            Placement::SameSet => together.push(sibling.coordinate.clone()),
            Placement::OwnSet => alone.push(sibling.coordinate.clone()),
        }
    }
    publish(rig, &mut p, &Set::at(rig, together));
    for coordinate in alone {
        publish(rig, &mut p, &Set::at(rig, vec![coordinate]));
    }
    let both = rig.grant(
        &p,
        GrantPhase::Commit,
        &[a.clone(), platform.coordinate.clone()],
    );
    must(p.yank(&both, &a, "a reason"), "a yank by version");
    same(
        is_yanked(&looked(rig, shape, &mut p, &platform.coordinate)),
        true,
        &format!("the platform a yank by version withdrew ({shape:?})"),
    );
    present_held(looked(rig, shape, &mut p, &version.coordinate));
    same(
        fetched(rig, &mut p, &version.coordinate),
        Some(rig.payload(&version.coordinate, 0).bytes),
        &format!("the bytes of another version, which a yank by version leaves ({shape:?})"),
    );
}

/// A yank names one coordinate: the others that were committed in the same call stay as they
/// were, and so does a coordinate whose name extends the name it names.
fn a_yank_names_a_coordinate_and_not_what_was_committed_with_it_or_what_extends_its_name<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let together = Set::of(rig, &["a", "b"]);
    let longer = Set::of(rig, &["ab"]);
    publish(rig, &mut p, &together);
    publish(rig, &mut p, &longer);
    let grant = rig.grant(&p, GrantPhase::Commit, &together.coords[..1]);
    must(p.yank(&grant, &together.coords[0], "a reason"), "yank");
    same(
        is_yanked(&looked(rig, shape, &mut p, &together.coords[0])),
        true,
        "the coordinate a yank named",
    );
    for (what, kept) in [
        ("committed in the same call", &together.coords[1]),
        ("whose name extends the name", &longer.coords[0]),
    ] {
        present_held(looked(rig, shape, &mut p, kept));
        same(
            fetched(rig, &mut p, kept).is_some(),
            true,
            &format!("the bytes of a coordinate {what} as the one a yank named"),
        );
    }
}

/// A yank by version: a grant that names the coordinate and not the platform of it that is held
/// is `Denied` and changes nothing, and a grant that names both yanks both.
fn yank_a_version<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
    p: &mut R::Publisher,
    a: &Coordinate,
    sibling: &Coordinate,
) {
    let alone = rig.grant(p, GrantPhase::Commit, std::slice::from_ref(a));
    same(
        p.yank(&alone, a, "a reason"),
        Err(SeamError::GrantRefused(GrantFault::Denied)),
        &format!("a yank by version whose grant leaves a held platform out ({shape:?})"),
    );
    present_held(looked(rig, shape, p, a));
    present_held(looked(rig, shape, p, sibling));
    let both = rig.grant(p, GrantPhase::Commit, &[a.clone(), sibling.clone()]);
    must(p.yank(&both, a, "a reason"), "a yank by version");
    same(
        is_yanked(&looked(rig, shape, p, a)),
        true,
        &format!("the coordinate a yank by version named ({shape:?})"),
    );
    same(
        is_yanked(&looked(rig, shape, p, sibling)),
        true,
        &format!(
            "the platform of it that was held, which a yank by version also withdraws ({shape:?})"
        ),
    );
}
