// SPDX-License-Identifier: Apache-2.0
//! Scenarios of the publisher kit about what a call that was refused, or answered `Duplicate`,
//! leaves behind; about a commit and what was staged; about sets; and about coordinates of one
//! name that differ in a field.

use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::Presence;
use ucdk_release_proto::v1::Stamped;

use super::publisher::{
    Arrangement, ChannelShape, PublisherRig, Set, commit, fetched, observed, present_held, publish,
    stage, staged_held, stranger,
};
use super::publisher_grants::differing_in_one_field;
use super::{coordinate, must, must_err, same};
use crate::{CommitAnswer, ListScope, Publisher, StageAnswer};

/// A commit carries what was staged: other bytes, another stamp, or a coordinate nobody staged
/// on a channel that stages are each refused, and the staged state, which a commit of what was
/// staged still makes readable, is as it was.
pub(super) fn a_commit_carries_exactly_what_was_staged<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    stage(rig, &mut p, &set);
    let mut other_bytes = Set::of(&["a"]);
    other_bytes.bytes[0] = b"other".to_vec();
    let mut other_stamp = Set::of(&["a"]);
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
    let unstaged = Set::of(&["u"]);
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
        Some(b"a".to_vec()),
        "the bytes of what was staged, committed after the refusals",
    );
}

/// A stage or a commit of a set one coordinate of which is held by another writer answers
/// `Duplicate` and applies none of the set: the others are not staged, not committed.
pub(super) fn a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was<R: PublisherRig>(
    rig: &R,
) {
    let held_by_another = || Arrangement::HeldByAnother {
        coordinate: coordinate("a"),
        bytes: b"theirs".to_vec(),
    };
    let mut p = rig.publisher(&ChannelShape::staging());
    rig.arrange(&mut p, &held_by_another());
    let two = Set::of(&["c", "a"]);
    let blobs = two.blobs();
    let items = two.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Stage, &two.coords);
    same(
        must(p.stage(&grant, &items), "a stage of a set"),
        StageAnswer::Duplicate,
        "a stage of a set one coordinate of which is held",
    );
    let free = Set::of(&["c"]);
    same(
        observed(rig, &mut p, GrantPhase::Stage, &free, 0),
        Presence::Absent,
        "the free coordinate of a set whose stage was answered Duplicate",
    );
    for shape in [ChannelShape::staging(), ChannelShape::without_staging()] {
        let mut p = rig.publisher(&shape);
        rig.arrange(&mut p, &held_by_another());
        let b = Set::of(&["b"]);
        if shape.stages {
            stage(rig, &mut p, &b);
        }
        // The held coordinate is the last of the set: a registry that looks at the first alone
        // is held to the answer for the whole.
        let two = Set::of(&["b", "a"]);
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
        same(
            fetched(rig, &mut p, &two.coords[1]),
            Some(b"theirs".to_vec()),
            &format!("the held coordinate after a commit of a set ({shape:?})"),
        );
        if shape.stages {
            staged_held(observed(rig, &mut p, GrantPhase::Stage, &b, 0));
        }
    }
}

/// A set that names one coordinate twice could carry two byte strings for it: a stage, a commit
/// and an observe refuse it and change nothing.
pub(super) fn a_set_that_names_a_coordinate_twice_is_refused<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let a = coordinate("a");
    let mut twice = Set::at(vec![a.clone(), a.clone()]);
    twice.bytes[1] = b"other".to_vec();
    let blobs = twice.blobs();
    let items = twice.items(&blobs);
    let stage_grant = rig.grant(&p, GrantPhase::Stage, std::slice::from_ref(&a));
    must_err(
        p.stage(&stage_grant, &items),
        "a stage that names a coordinate twice",
    );
    let once = Set::of(&["a"]);
    same(
        observed(rig, &mut p, GrantPhase::Stage, &once, 0),
        Presence::Absent,
        "the coordinate after a refused stage of a set that names it twice",
    );
    must_err(
        p.observe(&stage_grant, &a, &items),
        "an observe told a set that names a coordinate twice",
    );
    // Staged with the bytes of the last of the two, so that a commit that keeps the last of them
    // would be one of what was staged.
    let mut last = Set::of(&["a"]);
    last.bytes[0] = b"other".to_vec();
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
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &last, 0));
}

/// Staged state is no more readable to the one who staged it than to a stranger, but for an
/// observe: a stage grant and a commit grant fetch nothing of it and find nothing of it with
/// `newest` or `list`.
pub(super) fn a_stage_or_commit_grant_reads_no_staged_state_but_through_observe<R: PublisherRig>(
    rig: &R,
) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    stage(rig, &mut p, &set);
    let a = &set.coords[0];
    let scope = ListScope {
        channel: a.channel.clone(),
        prefix: String::new(),
    };
    for phase in [GrantPhase::Stage, GrantPhase::Commit] {
        let grant = rig.grant(&p, phase, &set.coords);
        same(
            must(p.fetch(&grant, a), "a fetch with a writer's grant"),
            None,
            &format!("the bytes of a staged coordinate fetched with a {phase:?} grant"),
        );
        same(
            must(
                p.newest(&grant, &a.channel, &a.name),
                "a newest with a writer's grant",
            )
            .is_empty(),
            true,
            &format!("a staged coordinate found with newest and a {phase:?} grant"),
        );
        same(
            must(p.list(&grant, &scope), "a list with a writer's grant").is_empty(),
            true,
            &format!("a staged coordinate listed with a {phase:?} grant"),
        );
    }
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
}

/// On a channel that does not stage, a coordinate is created once too: a commit that lost a
/// race leaves the bytes that won, a commit over a yanked coordinate is not a commit, a failure
/// after a commit that landed is not `Permanent`, and a set commit that fails makes nothing
/// readable.
pub(super) fn a_channel_that_does_not_stage_creates_a_coordinate_once<R: PublisherRig>(rig: &R) {
    let shape = ChannelShape::without_staging();
    let set = Set::of(&["a"]);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let a = &set.coords[0];

    let mut p = rig.publisher(&shape);
    rig.arrange(&mut p, &Arrangement::RaceNextCommit(b"theirs".to_vec()));
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    same(
        must(p.commit(&grant, &items), "a commit that lost a race"),
        CommitAnswer::Duplicate,
        "the answer to a commit that lost a race on a channel that does not stage",
    );
    same(
        fetched(rig, &mut p, a),
        Some(b"theirs".to_vec()),
        "the bytes after a race on a channel that does not stage",
    );

    let mut p = rig.publisher(&shape);
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
        matches!(stranger(&mut p, a), Presence::Yanked(_)),
        true,
        "the coordinate after the second commit, on a channel that does not stage",
    );

    let mut p = rig.publisher(&shape);
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
    present_held(stranger(&mut p, a));

    let mut p = rig.publisher(&shape);
    let two = Set::of(&["a", "b"]);
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

/// A coordinate that differs from another of its name in one field is another coordinate: it is
/// staged and committed beside a held one, and an observe of it finds nothing of the staged
/// state of the other.
pub(super) fn a_sibling_is_another_coordinate_to_stage_commit_and_observe<R: PublisherRig>(
    rig: &R,
) {
    for (field, sibling) in differing_in_one_field(&coordinate("a")) {
        let mut p = rig.publisher(&ChannelShape::staging());
        let held = Set::at(vec![coordinate("a")]);
        publish(rig, &mut p, &held);
        let beside = Set::at(vec![sibling.clone()]);
        stage(rig, &mut p, &beside);
        commit(rig, &mut p, &beside);
        present_held(stranger(&mut p, &sibling));
        same(
            fetched(rig, &mut p, &sibling),
            Some(sibling.name.as_bytes().to_vec()),
            &format!("the bytes of a coordinate beside one that differs in its {field}"),
        );

        let mut p = rig.publisher(&ChannelShape::staging());
        stage(rig, &mut p, &held);
        let grant = rig.grant(&p, GrantPhase::Stage, std::slice::from_ref(&sibling));
        same(
            must(
                p.observe(&grant, &sibling, &[]),
                "an observe of a sibling of a staged coordinate",
            ),
            Presence::Absent,
            &format!("a coordinate that differs in its {field} from one that is staged"),
        );
    }
}

/// An abort gives back the staged state of the coordinates it names and of no sibling of theirs
/// that differs in a field; what it gave back can be staged again.
pub(super) fn an_abort_reaches_only_the_coordinates_it_names<R: PublisherRig>(rig: &R) {
    for (_, sibling) in differing_in_one_field(&coordinate("a")) {
        let mut p = rig.publisher(&ChannelShape::staging());
        let (named, other) = (
            Set::at(vec![coordinate("a")]),
            Set::at(vec![sibling.clone()]),
        );
        stage(
            rig,
            &mut p,
            &Set::at(vec![coordinate("a"), sibling.clone()]),
        );
        let grant = rig.grant(&p, GrantPhase::Stage, &named.coords);
        must(p.abort(&grant, &named.coords), "abort");
        same(
            observed(rig, &mut p, GrantPhase::Stage, &named, 0),
            Presence::Absent,
            "the coordinate an abort named",
        );
        staged_held(observed(rig, &mut p, GrantPhase::Stage, &other, 0));
        stage(rig, &mut p, &named);
    }
}

/// A yank yanks the coordinate it names and no sibling of it, and a coordinate nobody holds is
/// no coordinate to yank.
pub(super) fn a_yank_reaches_only_the_coordinate_it_names_and_needs_it_held<R: PublisherRig>(
    rig: &R,
) {
    for (_, sibling) in differing_in_one_field(&coordinate("a")) {
        let mut p = rig.publisher(&ChannelShape::staging());
        let a = coordinate("a");
        publish(rig, &mut p, &Set::at(vec![a.clone(), sibling.clone()]));
        let grant = rig.grant(&p, GrantPhase::Commit, std::slice::from_ref(&a));
        must(p.yank(&grant, &a, "a reason"), "yank");
        same(
            matches!(stranger(&mut p, &a), Presence::Yanked(_)),
            true,
            "the coordinate a yank named",
        );
        present_held(stranger(&mut p, &sibling));
    }
    let mut p = rig.publisher(&ChannelShape::staging());
    let nothing = coordinate("x");
    let grant = rig.grant(&p, GrantPhase::Commit, std::slice::from_ref(&nothing));
    must_err(
        p.yank(&grant, &nothing, "a reason"),
        "a yank of a coordinate nobody holds",
    );
    same(
        stranger(&mut p, &nothing),
        Presence::Absent,
        "a coordinate nobody holds after a yank of it",
    );
}
