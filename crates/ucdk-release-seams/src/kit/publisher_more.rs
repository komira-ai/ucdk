// SPDX-License-Identifier: Apache-2.0
//! Scenarios of the publisher kit about what a call that was refused, or answered `Duplicate`,
//! leaves behind; about a commit and what was staged; about sets; and about coordinates of one
//! name that differ in a field.

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::Presence;
use ucdk_release_proto::v1::{Coordinate, Stamped};

use super::publisher_rig::{
    Arrangement, ChannelShape, PublisherRig, Set, built, commit, fetched, fixture_publisher,
    held_by_another, is_yanked, observed, plain_shapes, present_held, publish, sets_beside,
    siblings_beside, stage, staged_held, staging_publisher, stranger, unstaged_publisher,
};
use super::{must, must_err, same};
use crate::{CommitAnswer, CommitUnit, ListScope, Publisher, SeamError, StageAnswer, YankUnit};

/// A commit carries what was staged: other bytes, another stamp, or a coordinate nobody staged
/// on a channel that stages are each refused, and the staged state, which a commit of what was
/// staged still makes readable, is as it was.
pub(super) fn a_commit_carries_exactly_what_was_staged<R: PublisherRig>(rig: &R) {
    let mut p = staging_publisher(rig);
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
}

/// A stage or a commit of a set one coordinate of which is held by another writer answers
/// `Duplicate` and applies none of the set: the others are not staged, not committed. Run on
/// every declared channel that commits a set at once.
pub(super) fn a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was<R: PublisherRig>(
    rig: &R,
) {
    let shapes: Vec<ChannelShape> = plain_shapes(rig)
        .into_iter()
        .filter(|shape| built(rig, shape).commit_unit() == CommitUnit::Set)
        .collect();
    for shape in shapes {
        if shape.stages {
            a_stage_of_a_set_that_meets_a_held_coordinate(rig, &shape);
        }
        a_commit_of_a_set_that_meets_a_held_coordinate(rig, &shape);
    }
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
    same(
        fetched(rig, &mut p, &two.coords[1]),
        Some(set_of_another(rig, &two.coords[1])),
        &format!("the held coordinate after a commit of a set ({shape:?})"),
    );
    if shape.stages {
        staged_held(observed(rig, &mut p, GrantPhase::Stage, &b, 0));
    }
}

/// The bytes another writer's member of `k` holds.
fn set_of_another<R: PublisherRig>(rig: &R, k: &Coordinate) -> Vec<u8> {
    rig.payload(k, 1).bytes
}

/// A set that names one coordinate twice could carry two byte strings for it: a stage, a commit
/// and an observe refuse it and change nothing.
pub(super) fn a_set_that_names_a_coordinate_twice_is_refused<R: PublisherRig>(rig: &R) {
    let mut p = staging_publisher(rig);
    let a = rig.coordinate("a");
    let mut twice = Set::at(rig, vec![a.clone(), a.clone()]);
    twice.payloads[1] = rig.payload(&a, 1);
    let blobs = twice.blobs();
    let items = twice.items(&blobs);
    let stage_grant = rig.grant(&p, GrantPhase::Stage, std::slice::from_ref(&a));
    must_err(
        p.stage(&stage_grant, &items),
        "a stage that names a coordinate twice",
    );
    let once = Set::of(rig, &["a"]);
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
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &last, 0));
}

/// Staged state is no more readable to the one who staged it than to a stranger, but for an
/// observe: a stage grant and a commit grant fetch nothing of it and find nothing of it with
/// `newest` or `list`.
pub(super) fn a_stage_or_commit_grant_reads_no_staged_state_but_through_observe<R: PublisherRig>(
    rig: &R,
) {
    let mut p = staging_publisher(rig);
    let set = Set::of(rig, &["a"]);
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
    let set = Set::of(rig, &["a"]);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let a = &set.coords[0];

    let mut p = unstaged_publisher(rig);
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

    let mut p = unstaged_publisher(rig);
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
        is_yanked(&stranger(&mut p, a)),
        true,
        "the coordinate after the second commit, on a channel that does not stage",
    );

    let mut p = unstaged_publisher(rig);
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

    let mut p = unstaged_publisher(rig);
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
) {
    let base = rig.coordinate("a");
    for sibling in siblings_beside(rig, &base) {
        let field = sibling.field;
        let mut p = fixture_publisher(rig);
        for set in sets_beside(rig, &base, &sibling) {
            publish(rig, &mut p, &set);
        }
        present_held(stranger(&mut p, &base));
        present_held(stranger(&mut p, &sibling.coordinate));
        same(
            fetched(rig, &mut p, &sibling.coordinate),
            Some(rig.payload(&sibling.coordinate, 0).bytes),
            &format!("the bytes of a coordinate beside one that differs in its {field}"),
        );

        let mut p = fixture_publisher(rig);
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
/// that differs in a field; what it gave back can be staged again.
pub(super) fn an_abort_reaches_only_the_coordinates_it_names<R: PublisherRig>(rig: &R) {
    let base = rig.coordinate("a");
    for sibling in siblings_beside(rig, &base) {
        let mut p = staging_publisher(rig);
        let named = Set::at(rig, vec![base.clone()]);
        let other = Set::at(rig, vec![sibling.coordinate.clone()]);
        for set in sets_beside(rig, &base, &sibling) {
            stage(rig, &mut p, &set);
        }
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

/// A yank yanks the coordinate it names and no sibling of it, unless the channel's `yank_unit()`
/// is the version, which is held both ways: a channel that yanks a version yanks every held
/// platform of it when the grant covers them all and refuses, changing nothing, when the grant
/// leaves one out; a channel that yanks a coordinate leaves its sibling alone. A coordinate
/// nobody holds is no coordinate to yank.
pub(super) fn a_yank_reaches_only_the_coordinate_it_names_and_needs_it_held<R: PublisherRig>(
    rig: &R,
) {
    let a = rig.coordinate("a");
    for sibling in siblings_beside(rig, &a) {
        let mut p = fixture_publisher(rig);
        for set in sets_beside(rig, &a, &sibling) {
            publish(rig, &mut p, &set);
        }
        let of_the_version = sibling.field == "platform" && p.yank_unit() == YankUnit::Version;
        if of_the_version {
            yank_a_version(rig, &mut p, &a, &sibling.coordinate);
        } else {
            let grant = rig.grant(&p, GrantPhase::Commit, std::slice::from_ref(&a));
            must(p.yank(&grant, &a, "a reason"), "yank");
            same(
                is_yanked(&stranger(&mut p, &a)),
                true,
                "the coordinate a yank named",
            );
            present_held(stranger(&mut p, &sibling.coordinate));
        }
    }
    let mut p = fixture_publisher(rig);
    let nothing = rig.coordinate("x");
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

/// A yank by version: a grant that names the coordinate and not the platform of it that is held
/// is `Denied` and changes nothing, and a grant that names both yanks both.
fn yank_a_version<R: PublisherRig>(
    rig: &R,
    p: &mut R::Publisher,
    a: &Coordinate,
    sibling: &Coordinate,
) {
    let alone = rig.grant(p, GrantPhase::Commit, std::slice::from_ref(a));
    same(
        p.yank(&alone, a, "a reason"),
        Err(SeamError::GrantRefused(GrantFault::Denied)),
        "a yank by version whose grant leaves a held platform out",
    );
    present_held(stranger(p, a));
    present_held(stranger(p, sibling));
    let both = rig.grant(p, GrantPhase::Commit, &[a.clone(), sibling.clone()]);
    must(p.yank(&both, a, "a reason"), "a yank by version");
    same(
        is_yanked(&stranger(p, a)),
        true,
        "the coordinate a yank by version named",
    );
    same(
        is_yanked(&stranger(p, sibling)),
        true,
        "the platform of it that was held, which a yank by version also withdraws",
    );
}
