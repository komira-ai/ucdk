// SPDX-License-Identifier: Apache-2.0
//! The publisher kit: the scenarios about what a registry holds and answers, about commits and
//! sets, about who may read and write a channel, grants and private or grouped channels, and
//! about the coordinates and channels a rig declares.
//!
//! A rig declares the shapes of channel it builds and the hooks that say what its grammar takes
//! (`PublisherRig`). A scenario about what a channel does runs on every declared shape that can
//! do it, and one that needs a capability the rig declares absent is gated: the runner records it
//! as skipped with the [`Need`](super::Need) the rig lacks, and a skip is never a pass
//! (`publisher_gates`). The scenarios are in the order of the design (`publisher_list`): the first
//! sixteen about what a registry holds, the sixteen about grants and access that follow them
//! (`publisher_access`, `publisher_grants` and `publisher_private`), and then those about
//! commits, sets and the declarations of a rig.

use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::Stamped;

use super::publisher_grants::denied;
use super::publisher_list::publisher_scenarios;
use super::publisher_more as more;
use super::publisher_rig::{
    Arrangement, ChannelShape, PublisherRig, Set, built, commit, fetched, held_by_another,
    is_yanked, looked, observed, observed_as_staged, present_held, publish, stage, staged_held,
    stranger,
};
use super::{Report, must, must_err, run_scenarios, same};
use crate::{CommitAnswer, CommitUnit, Grant, ListScope, Publisher, StageAnswer};

pub fn run_publisher_kit<R: PublisherRig>(rig: &R) -> Report {
    run_scenarios(rig, &publisher_scenarios())
}

/// A coordinate is created once: a commit of the bytes it already holds is `Duplicate`, on a
/// channel that stages and on one that does not, and the coordinate is as it was. A commit of
/// staged state that lost a race is `Duplicate` too.
pub(super) fn a_second_commit_of_the_same_bytes_is_duplicate<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    publish(rig, &mut p, &set);
    let before = present_held(looked(rig, shape, &mut p, &set.coords[0]));
    let mut restamped = set.clone();
    restamped.held.stamp = Some(Stamped {
        build_number: 42,
        source_commit: "def456".to_owned(),
    });
    let mut marked = set.clone();
    marked.held.exceptional = true;
    for (what, again) in [
        ("the same bytes", &set),
        ("the same bytes and another stamp", &restamped),
        ("the same bytes and another mark", &marked),
    ] {
        let blobs = again.blobs();
        let items = again.items(&blobs);
        let grant = rig.grant(&p, GrantPhase::Commit, &again.coords);
        same(
            must(p.commit(&grant, &items), "a second commit"),
            CommitAnswer::Duplicate,
            &format!("a second commit of {what} ({shape:?})"),
        );
        same(
            present_held(looked(rig, shape, &mut p, &set.coords[0])),
            before.clone(),
            &format!("what the coordinate holds after a second commit of {what}"),
        );
    }
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(set.bytes(0).to_vec()),
        "the bytes after the second commit",
    );
    if shape.stages {
        a_commit_that_lost_a_race_leaves_the_bytes_that_won(rig, shape);
    }
}

/// On a channel that stages, a commit of staged state that lost a race is `Duplicate` and leaves
/// the bytes that won: having staged a coordinate is no licence to replace one that another
/// writer committed before the commit landed.
fn a_commit_that_lost_a_race_leaves_the_bytes_that_won<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    stage(rig, &mut p, &set);
    let theirs = set.reseeded(rig, 1);
    rig.arrange(
        &mut p,
        &Arrangement::RaceNextCommit(theirs.bytes(0).to_vec()),
    );
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    same(
        must(p.commit(&grant, &items), "a commit that lost a race"),
        CommitAnswer::Duplicate,
        &format!("the answer to a commit of staged state that lost a race ({shape:?})"),
    );
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(theirs.bytes(0).to_vec()),
        "the bytes after a race on a channel that stages",
    );
    present_held(looked(rig, shape, &mut p, &set.coords[0]));
}

/// A stage over a coordinate the caller itself has staged is `Duplicate` too, with the same
/// bytes and with other bytes, and the staged bytes are the first ones.
pub(super) fn a_stage_over_ones_own_staged_coordinate_is_duplicate<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    stage(rig, &mut p, &set);
    let other = set.reseeded(rig, 1);
    for (what, again) in [("the same bytes", &set), ("other bytes", &other)] {
        let blobs = again.blobs();
        let items = again.items(&blobs);
        let grant = rig.grant(&p, GrantPhase::Stage, &again.coords);
        same(
            must(p.stage(&grant, &items), "a second stage"),
            StageAnswer::Duplicate,
            &format!("a stage over one's own staged coordinate with {what} ({shape:?})"),
        );
    }
    commit(rig, &mut p, &set);
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(set.bytes(0).to_vec()),
        "the bytes committed after the second stages",
    );
}

pub(super) fn observe_absent_is_not_an_error<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    same(
        looked(rig, shape, &mut p, &set.coords[0]),
        Presence::Absent,
        &format!("a coordinate nobody wrote ({shape:?})"),
    );
    a_held_coordinate_does_not_answer_for_one_that_differs_in_a_field(rig, shape);
    a_coordinate_whose_name_extends_the_one_asked_for_is_another_coordinate(rig, shape);
    more::a_sibling_is_another_coordinate_to_stage_commit_and_observe(rig, shape);
}

/// A name is the whole of the name: a held coordinate whose name extends the name asked for is
/// not the coordinate asked for, to `observe` or to `fetch`.
fn a_coordinate_whose_name_extends_the_one_asked_for_is_another_coordinate<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let longer = Set::of(rig, &["ab"]);
    publish(rig, &mut p, &longer);
    let shorter = rig.coordinate("a");
    same(
        looked(rig, shape, &mut p, &shorter),
        Presence::Absent,
        "a coordinate whose name a held one extends",
    );
    let grant = rig.grant(&p, GrantPhase::Read, std::slice::from_ref(&shorter));
    same(
        must(
            p.observe(&grant, &shorter, &[]),
            "a granted observe of a coordinate whose name a held one extends",
        ),
        Presence::Absent,
        "a granted observe of a coordinate whose name a held one extends",
    );
    same(
        fetched(rig, &mut p, &shorter),
        None,
        "the bytes of a coordinate whose name a held one extends",
    );
}

/// A coordinate is the whole of its five fields: the one held that differs from the one asked
/// for in a field is not it, to `observe` or to `fetch`.
fn a_held_coordinate_does_not_answer_for_one_that_differs_in_a_field<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let held = rig.coordinate("a");
    publish(rig, &mut p, &Set::at(rig, vec![held.clone()]));
    present_held(looked(rig, shape, &mut p, &held));
    for sibling in rig.siblings(&held) {
        let (field, asked) = (sibling.field, &sibling.coordinate);
        same(
            looked(rig, shape, &mut p, asked),
            Presence::Absent,
            &format!("a coordinate that differs from a held one in its {field}"),
        );
        let grant = rig.grant(&p, GrantPhase::Read, std::slice::from_ref(asked));
        same(
            must(
                p.observe(&grant, asked, &[]),
                "an observe with a grant of a coordinate that differs",
            ),
            Presence::Absent,
            &format!("a granted observe of a coordinate that differs in its {field}"),
        );
        same(
            fetched(rig, &mut p, asked),
            None,
            &format!("the bytes of a coordinate that differs from a held one in its {field}"),
        );
    }
}

pub(super) fn stage_then_observe_returns_staged_with_the_same_digest<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    stage(rig, &mut p, &set);
    let held = staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    same(
        held.digest,
        set.digest(0),
        &format!("the digest of the staged coordinate ({shape:?})"),
    );
}

/// A stranger finds nothing of a staged coordinate on a public channel, and on a private one is
/// turned away: the refusal is `Denied`, and it does not say whether the coordinate is staged.
pub(super) fn a_staged_coordinate_is_not_readable_by_a_stranger<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    stage(rig, &mut p, &set);
    if shape.private {
        let refusal = must_err(
            p.observe(&Grant::none(), &set.coords[0], &[]),
            "an observe of a staged coordinate by a stranger of a private channel",
        );
        same(
            denied(&refusal),
            true,
            &format!("a stranger of a private channel is Denied, got {refusal:?} ({shape:?})"),
        );
    } else {
        same(
            stranger(&mut p, &set.coords[0]),
            Presence::Absent,
            &format!("a staged coordinate seen by a stranger ({shape:?})"),
        );
    }
}

pub(super) fn a_stage_grant_observes_the_staged_state_it_made<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    stage(rig, &mut p, &set);
    staged_held(observed_as_staged(
        rig,
        shape,
        &mut p,
        GrantPhase::Stage,
        &set,
        0,
    ));
}

pub(super) fn a_commit_grant_observes_it_too<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    stage(rig, &mut p, &set);
    staged_held(observed_as_staged(
        rig,
        shape,
        &mut p,
        GrantPhase::Commit,
        &set,
        0,
    ));
}

pub(super) fn commit_makes_the_staged_coordinates_readable_with_the_same_digest<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let mut set = Set::of(rig, &["a", "b"]);
    set.held.exceptional = true;
    publish(rig, &mut p, &set);
    for (at, coord) in set.coords.iter().enumerate() {
        let held = present_held(looked(rig, shape, &mut p, coord));
        same(
            held,
            Held {
                digest: set.digest(at),
                ..set.held.clone()
            },
            &format!(
                "what the member {at} of a committed set holds: the digest of the bytes sent, the stamp and the mark ({shape:?})"
            ),
        );
        same(
            fetched(rig, &mut p, coord),
            Some(set.bytes(at).to_vec()),
            &format!(
                "the bytes a read grant fetches of the member {at} of a committed set ({shape:?})"
            ),
        );
    }
}

pub(super) fn stage_of_present_same_is_duplicate_and_changes_nothing<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    publish(rig, &mut p, &set);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Stage, &set.coords);
    same(
        must(p.stage(&grant, &items), "stage"),
        StageAnswer::Duplicate,
        &format!("a stage over the same bytes ({shape:?})"),
    );
    present_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(set.bytes(0).to_vec()),
        "the bytes after it",
    );
}

/// Runs only for a channel that is a conditional put (release.md 4.3): its commit never replaces
/// what is there, whoever wrote it.
pub(super) fn commit_over_present_different_leaves_old_bytes<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    same(
        p.conditional_put(),
        true,
        "conditional_put() of the channel this scenario runs on",
    );
    let set = Set::of(rig, &["a"]);
    rig.arrange(&mut p, &held_by_another(rig, &set.coords[0]));
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    same(
        must(p.commit(&grant, &items), "commit"),
        CommitAnswer::Duplicate,
        &format!("a commit over other bytes ({shape:?})"),
    );
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(set.reseeded(rig, 1).bytes(0).to_vec()),
        "the bytes after it",
    );
}

pub(super) fn abort_gives_staged_state_back_and_never_touches_a_committed_coordinate<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let (a, b, c) = (
        Set::of(rig, &["a"]),
        Set::of(rig, &["b"]),
        Set::of(rig, &["c"]),
    );
    publish(rig, &mut p, &a);
    stage(rig, &mut p, &b);
    stage(rig, &mut p, &c);
    let both = Set::of(rig, &["a", "b"]);
    let grant = rig.grant(&p, GrantPhase::Stage, &both.coords);
    must(p.abort(&grant, &both.coords), "abort");
    same(
        observed(rig, &mut p, GrantPhase::Stage, &b, 0),
        Presence::Absent,
        "the staged coordinate after the abort",
    );
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &c, 0));
    present_held(looked(rig, shape, &mut p, &a.coords[0]));
    same(
        fetched(rig, &mut p, &a.coords[0]),
        Some(a.bytes(0).to_vec()),
        "the committed bytes after it",
    );
    more::an_abort_reaches_only_the_coordinates_it_names(rig, shape);
}

/// What one commit call makes readable is what `commit_unit()` says, held both ways: a channel
/// that commits a set at once makes every coordinate readable or none, and one that commits each
/// coordinate on its own refuses a call that names two and makes nothing of it readable.
pub(super) fn a_set_commit_makes_every_coordinate_readable_or_none<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a", "b"]);
    match p.commit_unit() {
        CommitUnit::Set => {
            publish(rig, &mut p, &set);
            for coord in &set.coords {
                present_held(looked(rig, shape, &mut p, coord));
            }
            let mut failing = built(rig, shape);
            rig.arrange(&mut failing, &Arrangement::CommitSetFailsWhole);
            stage(rig, &mut failing, &set);
            let blobs = set.blobs();
            let items = set.items(&blobs);
            let grant = rig.grant(&failing, GrantPhase::Commit, &set.coords);
            must_err(failing.commit(&grant, &items), "a set commit that fails");
            for coord in &set.coords {
                same(
                    fetched(rig, &mut failing, coord),
                    None,
                    "the bytes of a coordinate of a set that failed",
                );
            }
            more::a_set_commit_that_is_refused_applies_none_of_it(rig, shape);
        }
        CommitUnit::Coordinate => {
            stage(rig, &mut p, &set);
            let blobs = set.blobs();
            let items = set.items(&blobs);
            let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
            must_err(
                p.commit(&grant, &items),
                "a commit of two coordinates on a channel that commits each on its own",
            );
            for coord in &set.coords {
                same(
                    fetched(rig, &mut p, coord),
                    None,
                    "the bytes of a coordinate of a call that was refused",
                );
            }
            commit(rig, &mut p, &set);
            for coord in &set.coords {
                present_held(looked(rig, shape, &mut p, coord));
            }
        }
    }
}

pub(super) fn a_channel_that_does_not_stage_commits_with_the_bytes<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    same(
        p.stages(),
        false,
        "stages() of a channel that does not stage",
    );
    let set = Set::of(rig, &["a"]);
    commit(rig, &mut p, &set);
    present_held(looked(rig, shape, &mut p, &set.coords[0]));
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(set.bytes(0).to_vec()),
        "the bytes committed",
    );
}

pub(super) fn no_digest_exposed_is_refused<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    rig.arrange(&mut p, &held_by_another(rig, &set.coords[0]));
    let held = present_held(looked(rig, shape, &mut p, &set.coords[0]));
    same(
        held.digest,
        set.reseeded(rig, 1).digest(0),
        &format!("the digest a coordinate of another writer is reported with ({shape:?})"),
    );
}

pub(super) fn yank_keeps_bytes_fetchable<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    publish(rig, &mut p, &set);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    must(p.yank(&grant, &set.coords[0], "a reason"), "yank");
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(set.bytes(0).to_vec()),
        "the bytes of a yanked coordinate",
    );
    same(
        is_yanked(&looked(rig, shape, &mut p, &set.coords[0])),
        true,
        "a yanked coordinate is observed as Yanked",
    );
    more::a_yank_reaches_only_the_coordinate_it_names_and_needs_it_held(rig, shape);
}

pub(super) fn yanked_coordinate_cannot_be_committed_again<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    publish(rig, &mut p, &set);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    must(p.yank(&grant, &set.coords[0], "a reason"), "yank");
    let blobs = set.blobs();
    let items = set.items(&blobs);
    if p.stages() {
        let staged = p.stage(&rig.grant(&p, GrantPhase::Stage, &set.coords), &items);
        same(
            staged,
            Ok(StageAnswer::Duplicate),
            "a stage over a yanked coordinate",
        );
    }
    let again = p.commit(&grant, &items);
    same(
        matches!(again, Ok(CommitAnswer::Committed)),
        false,
        "a commit over a yanked coordinate",
    );
    same(
        is_yanked(&looked(rig, shape, &mut p, &set.coords[0])),
        true,
        "the coordinate after the second commit",
    );
}

/// A commit whose answer was lost is resolved by an observe. On a channel that stages, a commit
/// that did not land leaves the coordinate `Staged`; on one that does not, it leaves it `Absent`,
/// and the caller commits again. A commit that landed is `Present` on either.
pub(super) fn lost_answer_is_resolvable_by_observe<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let set = Set::of(rig, &["a"]);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let mut landed = built(rig, shape);
    stage(rig, &mut landed, &set);
    rig.arrange(&mut landed, &Arrangement::LoseAnswerAfterCommit);
    let grant = rig.grant(&landed, GrantPhase::Commit, &set.coords);
    let lost = must_err(
        landed.commit(&grant, &items),
        "a commit whose answer is lost",
    );
    same(
        lost.promises_no_effect(),
        false,
        "a lost answer may have landed",
    );
    present_held(looked(rig, shape, &mut landed, &set.coords[0]));
    let mut not_landed = built(rig, shape);
    stage(rig, &mut not_landed, &set);
    rig.arrange(&mut not_landed, &Arrangement::LoseAnswerBeforeCommit);
    let grant = rig.grant(&not_landed, GrantPhase::Commit, &set.coords);
    let lost = must_err(
        not_landed.commit(&grant, &items),
        "a commit that did not land",
    );
    same(
        lost.promises_no_effect(),
        false,
        "a lost answer may have landed",
    );
    if shape.stages {
        staged_held(observed(rig, &mut not_landed, GrantPhase::Commit, &set, 0));
    } else {
        same(
            observed(rig, &mut not_landed, GrantPhase::Commit, &set, 0),
            Presence::Absent,
            "a coordinate whose commit did not land, on a channel that does not stage",
        );
    }
}

pub(super) fn list_may_lag_but_observe_is_exact<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    rig.arrange(&mut p, &Arrangement::ListLags);
    let set = Set::of(rig, &["a"]);
    publish(rig, &mut p, &set);
    present_held(looked(rig, shape, &mut p, &set.coords[0]));
    let grant = rig.grant(&p, GrantPhase::Read, &set.coords);
    let listed = must(
        p.list(
            &grant,
            &ListScope {
                channel: set.coords[0].channel.clone(),
                prefix: String::new(),
            },
        ),
        "list",
    );
    same(
        listed.iter().all(|l| l.coordinate == set.coords[0]),
        true,
        "a listing names only what is held",
    );
    a_listing_names_what_is_held_in_its_scope(rig, shape);
    an_observe_of_a_coordinate_of_another_writer_is_exact_while_the_listing_lags(rig, shape);
}

/// What another writer committed is read by an observe as it is, however far behind the listing
/// is: the coordinate is `Present` with the digest of the bytes the other writer sent.
fn an_observe_of_a_coordinate_of_another_writer_is_exact_while_the_listing_lags<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    rig.arrange(&mut p, &Arrangement::ListLags);
    rig.arrange(&mut p, &held_by_another(rig, &set.coords[0]));
    same(
        present_held(looked(rig, shape, &mut p, &set.coords[0])).digest,
        set.reseeded(rig, 1).digest(0),
        "the digest of a coordinate another writer committed, while the listing lags",
    );
}

/// A listing that was not arranged to lag names every coordinate held in the channel under the
/// prefix of its scope, and what it names is held, in that channel and under that prefix, and
/// carries what the registry holds beside each. The prefix is a label the kit gave the rig, so the
/// rig's names must keep it.
fn a_listing_names_what_is_held_in_its_scope<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let mut p = built(rig, shape);
    let mut coords = vec![
        rig.coordinate("a"),
        rig.coordinate("ab"),
        rig.coordinate("b"),
    ];
    let beta = rig
        .siblings(&coords[0])
        .into_iter()
        .find(|s| s.field == "channel");
    let channels: Vec<(String, &str)> = match &beta {
        Some(s) => {
            coords.push(s.coordinate.clone());
            vec![
                (coords[0].channel.clone(), "a"),
                (coords[0].channel.clone(), ""),
                (s.coordinate.channel.clone(), "a"),
            ]
        }
        None => vec![
            (coords[0].channel.clone(), "a"),
            (coords[0].channel.clone(), ""),
        ],
    };
    let mut set = Set::at(rig, coords);
    set.held.exceptional = true;
    publish(rig, &mut p, &set);
    for (channel, prefix) in &channels {
        let scope = ListScope {
            channel: channel.clone(),
            prefix: (*prefix).to_owned(),
        };
        let grant = rig.grant(&p, GrantPhase::Read, &set.coords);
        let listed = must(p.list(&grant, &scope), "list");
        // A listing that was not arranged to lag names every coordinate held in its scope: a
        // registry whose listing is always empty is not one that lags, and a reader of a private
        // channel finds what is held as a reader of a public one does.
        for (at, held) in set.coords.iter().enumerate() {
            if (held.channel == *channel) & held.name.starts_with(prefix) {
                same(
                    listed.iter().any(|l| l.coordinate == *held),
                    true,
                    &format!(
                        "the coordinate {at} held in the channel {channel:?} under the prefix {prefix:?} is in the listing ({shape:?})"
                    ),
                );
            }
        }
        for entry in listed {
            let c = &entry.coordinate;
            let at = super::must_some(
                set.coords.iter().position(|held| held == c),
                &format!("{c:?}, listed, though the kit never wrote it"),
            );
            same(
                (c.channel == *channel) & c.name.starts_with(prefix),
                true,
                &format!("{c:?} listed for the channel {channel:?} and the prefix {prefix:?}"),
            );
            same(
                entry.held,
                Held {
                    digest: set.digest(at),
                    ..set.held.clone()
                },
                &format!("what a listing holds beside {c:?}"),
            );
        }
    }
}
