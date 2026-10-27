// SPDX-License-Identifier: Apache-2.0
//! The publisher kit: thirty-nine scenarios over observing, staging, committing, aborting and
//! reading the coordinates of a channel, and over the grants those calls carry.
//!
//! The scenarios of this file are about what a registry holds and answers; those of
//! `publisher_access` are about who may read and write it and what a failed call left behind.

use sha2::{Digest as _, Sha256};
use ucdk_core::Hash256;
use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::{Coordinate, Stamped};

use super::publisher_access as access;
use super::publisher_more as more;
use super::{Report, Scenario, coordinate, fail, must, must_err, run_scenarios, same};
use crate::{Blob, CommitAnswer, Grant, ListScope, Publisher, StageAnswer, StageItem};

/// What kind of channel a scenario wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelShape {
    /// The channel holds staged bytes that are not readable until the commit.
    pub stages: bool,
    /// Every read, a stranger's included, needs a grant.
    pub private: bool,
    /// Staged state is addressed by a group derived from the declared set, and an observe is a
    /// group read followed by an index read.
    pub group_addressed: bool,
}

impl ChannelShape {
    /// A public channel that stages, commits a set at once and addresses staged state by
    /// coordinate.
    pub const fn staging() -> Self {
        Self {
            stages: true,
            private: false,
            group_addressed: false,
        }
    }

    /// A public channel that does not stage: `commit` carries the bytes.
    pub const fn without_staging() -> Self {
        Self {
            stages: false,
            ..Self::staging()
        }
    }

    /// A private channel that addresses staged state by group.
    pub const fn private_grouped() -> Self {
        Self {
            stages: true,
            private: true,
            group_addressed: true,
        }
    }
}

/// A change to a world a scenario asks its rig to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrangement {
    /// `coordinate` is committed by another writer, with `bytes`.
    HeldByAnother {
        coordinate: Coordinate,
        bytes: Vec<u8>,
    },
    /// `coordinate` is staged by another writer, with `bytes`.
    StagedByAnother {
        coordinate: Coordinate,
        bytes: Vec<u8>,
    },
    /// Just before the next commit lands, another writer commits `bytes` at its first
    /// coordinate.
    RaceNextCommit(Vec<u8>),
    /// The next commit lands and its answer is lost.
    LoseAnswerAfterCommit,
    /// The next commit does not land and its answer is lost.
    LoseAnswerBeforeCommit,
    /// The next commit lands and then fails with a transient error.
    FailAfterCommit,
    /// A commit of a set fails whole: nothing becomes readable.
    CommitSetFailsWhole,
    /// `list` lags behind what was committed.
    ListLags,
    /// The group read of an observe fails.
    FailGroupRead,
    /// The index read of an observe fails.
    FailIndexRead,
}

/// One read an observe made on a channel that addresses staged state by group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStep {
    Group,
    Index,
}

/// What an implementation provides: a channel of a shape, the grants it accepts and the few
/// changes to its world a scenario needs.
pub trait PublisherRig {
    type Publisher: Publisher;

    /// A fresh, empty channel of `shape`.
    fn publisher(&self, shape: &ChannelShape) -> Self::Publisher;

    /// Makes `what` true of the channel.
    fn arrange(&self, publisher: &mut Self::Publisher, what: &Arrangement);

    /// A grant of `phase` for `coordinates` in this channel's repository, good now.
    fn grant(
        &self,
        publisher: &Self::Publisher,
        phase: GrantPhase,
        coordinates: &[Coordinate],
    ) -> Grant;

    /// A grant of `phase` for `coordinates` in another repository.
    fn foreign_grant(
        &self,
        publisher: &Self::Publisher,
        phase: GrantPhase,
        coordinates: &[Coordinate],
    ) -> Grant;

    /// Moves the clock the channel reads by `ms` milliseconds.
    fn advance(&self, ms: u64);

    /// The group and index reads the channel's observes made, in order. Empty for a channel
    /// that does not address staged state by group.
    fn reads(&self, publisher: &Self::Publisher) -> Vec<ReadStep>;
}

/// A day, longer than any grant lives.
pub(super) const A_DAY_MS: u64 = 86_400_000;

/// The coordinates, bytes and `Held` of a set a scenario publishes.
pub(super) struct Set {
    pub(super) coords: Vec<Coordinate>,
    pub(super) bytes: Vec<Vec<u8>>,
    pub(super) held: Held,
}

impl Set {
    /// One coordinate per name, whose bytes are the name.
    pub(super) fn of(names: &[&str]) -> Self {
        Self::at(names.iter().map(|n| coordinate(n)).collect())
    }

    /// The given coordinates, whose bytes are their names.
    pub(super) fn at(coords: Vec<Coordinate>) -> Self {
        Self {
            bytes: coords.iter().map(|c| c.name.as_bytes().to_vec()).collect(),
            coords,
            held: Held {
                digest: String::new(),
                stamp: Some(Stamped {
                    build_number: 41,
                    source_commit: "abc123".to_owned(),
                }),
                exceptional: false,
            },
        }
    }

    pub(super) fn blobs(&self) -> Vec<Blob<'_>> {
        self.coords
            .iter()
            .zip(&self.bytes)
            .map(|(c, bytes)| Blob {
                name: c.name.as_str(),
                bytes,
            })
            .collect()
    }

    pub(super) fn items<'a>(&'a self, blobs: &'a [Blob<'a>]) -> Vec<StageItem<'a>> {
        self.coords
            .iter()
            .zip(blobs)
            .map(|(coordinate, file)| StageItem {
                coordinate,
                file,
                held: &self.held,
            })
            .collect()
    }
}

/// The identity digest of `bytes`, as a registry reports it.
pub(super) fn sha(bytes: &[u8]) -> String {
    Hash256::from_sha256(&Sha256::digest(bytes).into()).to_string()
}

pub(super) fn staged_held(presence: Presence) -> Held {
    match presence {
        Presence::Staged(held) => held,
        other => fail(format!("expected Staged, got {other:?}")),
    }
}

pub(super) fn present_held(presence: Presence) -> Held {
    match presence {
        Presence::Present(held) => held,
        other => fail(format!("expected Present, got {other:?}")),
    }
}

/// Stages `set` with a stage grant.
pub(super) fn stage<R: PublisherRig>(rig: &R, p: &mut R::Publisher, set: &Set) {
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(p, GrantPhase::Stage, &set.coords);
    same(
        must(p.stage(&grant, &items), "stage"),
        StageAnswer::Staged,
        "the answer to a stage",
    );
}

/// Commits `set` with a commit grant, staged or not.
pub(super) fn commit<R: PublisherRig>(rig: &R, p: &mut R::Publisher, set: &Set) {
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(p, GrantPhase::Commit, &set.coords);
    same(
        must(p.commit(&grant, &items), "commit"),
        CommitAnswer::Committed,
        "the answer to a commit",
    );
}

/// Stages and then commits `set`.
pub(super) fn publish<R: PublisherRig>(rig: &R, p: &mut R::Publisher, set: &Set) {
    stage(rig, p, set);
    commit(rig, p, set);
}

/// What a stranger sees of `k`.
pub(super) fn stranger<P: Publisher>(p: &mut P, k: &Coordinate) -> Presence {
    must(
        p.observe(&Grant::none(), k, &[]),
        "an observe by a stranger",
    )
}

/// What an observe with a grant of `phase` for the whole of `set` sees of the coordinate at
/// `at`, told the declared set.
pub(super) fn observed<R: PublisherRig>(
    rig: &R,
    p: &mut R::Publisher,
    phase: GrantPhase,
    set: &Set,
    at: usize,
) -> Presence {
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(p, phase, &set.coords);
    must(
        p.observe(&grant, &set.coords[at], &items),
        "an observe with a grant",
    )
}

/// What an observe with a grant of `phase` for the whole of `set` sees of the coordinate at
/// `at`, told no set: a read by the coordinate alone.
pub(super) fn observed_alone<R: PublisherRig>(
    rig: &R,
    p: &mut R::Publisher,
    phase: GrantPhase,
    set: &Set,
    at: usize,
) -> Presence {
    let grant = rig.grant(p, phase, &set.coords);
    must(
        p.observe(&grant, &set.coords[at], &[]),
        "an observe with a grant and no set",
    )
}

/// The bytes a read grant fetches for `k`.
pub(super) fn fetched<R: PublisherRig>(
    rig: &R,
    p: &mut R::Publisher,
    k: &Coordinate,
) -> Option<Vec<u8>> {
    let grant = rig.grant(p, GrantPhase::Read, std::slice::from_ref(k));
    must(p.fetch(&grant, k), "fetch")
}

pub fn run_publisher_kit<R: PublisherRig>(rig: &R) -> Report {
    run_scenarios(rig, &publisher_scenarios())
}

pub fn publisher_scenarios<R: PublisherRig>() -> Vec<Scenario<R>> {
    vec![
        Scenario::new("observe_absent_is_not_an_error", observe_absent_is_not_an_error::<R>),
        Scenario::new(
            "stage_then_observe_returns_staged_with_the_same_digest",
            stage_then_observe_returns_staged_with_the_same_digest::<R>,
        ),
        Scenario::new(
            "a_staged_coordinate_is_not_readable_by_a_stranger",
            a_staged_coordinate_is_not_readable_by_a_stranger::<R>,
        ),
        Scenario::new(
            "a_stage_grant_observes_the_staged_state_it_made",
            a_stage_grant_observes_the_staged_state_it_made::<R>,
        ),
        Scenario::new("a_commit_grant_observes_it_too", a_commit_grant_observes_it_too::<R>),
        Scenario::new(
            "commit_makes_the_staged_coordinates_readable_with_the_same_digest",
            commit_makes_the_staged_coordinates_readable_with_the_same_digest::<R>,
        ),
        Scenario::new(
            "stage_of_present_same_is_duplicate_and_changes_nothing",
            stage_of_present_same_is_duplicate_and_changes_nothing::<R>,
        ),
        Scenario::new(
            "commit_over_present_different_leaves_old_bytes",
            commit_over_present_different_leaves_old_bytes::<R>,
        ),
        Scenario::new(
            "abort_gives_staged_state_back_and_never_touches_a_committed_coordinate",
            abort_gives_staged_state_back_and_never_touches_a_committed_coordinate::<R>,
        ),
        Scenario::new(
            "a_set_commit_makes_every_coordinate_readable_or_none",
            a_set_commit_makes_every_coordinate_readable_or_none::<R>,
        ),
        Scenario::new(
            "a_channel_that_does_not_stage_commits_with_the_bytes",
            a_channel_that_does_not_stage_commits_with_the_bytes::<R>,
        ),
        Scenario::new("no_digest_exposed_is_refused", no_digest_exposed_is_refused::<R>),
        Scenario::new("yank_keeps_bytes_fetchable", yank_keeps_bytes_fetchable::<R>),
        Scenario::new(
            "yanked_coordinate_cannot_be_committed_again",
            yanked_coordinate_cannot_be_committed_again::<R>,
        ),
        Scenario::new(
            "lost_answer_is_resolvable_by_observe",
            lost_answer_is_resolvable_by_observe::<R>,
        ),
        Scenario::new(
            "list_may_lag_but_observe_is_exact",
            list_may_lag_but_observe_is_exact::<R>,
        ),
        Scenario::new("public_read_needs_no_grant", access::public_read_needs_no_grant::<R>),
        Scenario::new(
            "a_grant_for_another_repository_sends_nothing",
            access::a_grant_for_another_repository_sends_nothing::<R>,
        ),
        Scenario::new(
            "a_grant_of_the_wrong_phase_is_denied_and_nothing_happens",
            access::a_grant_of_the_wrong_phase_is_denied_and_nothing_happens::<R>,
        ),
        Scenario::new(
            "an_expired_grant_is_refused_and_nothing_happens",
            access::an_expired_grant_is_refused_and_nothing_happens::<R>,
        ),
        Scenario::new(
            "newest_lists_every_held_coordinate_of_the_name",
            access::newest_lists_every_held_coordinate_of_the_name::<R>,
        ),
        Scenario::new(
            "observe_returns_the_stamp_it_was_given",
            access::observe_returns_the_stamp_it_was_given::<R>,
        ),
        Scenario::new(
            "a_granted_observe_sees_a_coordinate_committed_by_another_writer",
            access::a_granted_observe_sees_a_coordinate_committed_by_another_writer::<R>,
        ),
        Scenario::new(
            "a_private_observe_reads_the_group_then_the_index",
            access::a_private_observe_reads_the_group_then_the_index::<R>,
        ),
        Scenario::new(
            "a_coordinate_committed_in_another_group_reads_as_present_never_absent",
            access::a_coordinate_committed_in_another_group_reads_as_present_never_absent::<R>,
        ),
        Scenario::new(
            "a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent",
            access::a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent::<R>,
        ),
        Scenario::new(
            "observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set",
            access::observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set::<R>,
        ),
        Scenario::new(
            "observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates",
            access::observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates::<R>,
        ),
        Scenario::new(
            "a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit",
            access::a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit::<R>,
        ),
        Scenario::new(
            "commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit",
            access::commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit::<R>,
        ),
        Scenario::new(
            "transient_after_commit_is_observed_before_retry",
            access::transient_after_commit_is_observed_before_retry::<R>,
        ),
        Scenario::new(
            "duplicate_over_other_bytes_leaves_the_old_bytes",
            access::duplicate_over_other_bytes_leaves_the_old_bytes::<R>,
        ),
        Scenario::new(
            "a_second_commit_of_the_same_bytes_is_duplicate",
            a_second_commit_of_the_same_bytes_is_duplicate::<R>,
        ),
        Scenario::new(
            "a_stage_over_ones_own_staged_coordinate_is_duplicate",
            a_stage_over_ones_own_staged_coordinate_is_duplicate::<R>,
        ),
        Scenario::new(
            "a_commit_carries_exactly_what_was_staged",
            more::a_commit_carries_exactly_what_was_staged::<R>,
        ),
        Scenario::new(
            "a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was",
            more::a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was::<R>,
        ),
        Scenario::new(
            "a_set_that_names_a_coordinate_twice_is_refused",
            more::a_set_that_names_a_coordinate_twice_is_refused::<R>,
        ),
        Scenario::new(
            "a_stage_or_commit_grant_reads_no_staged_state_but_through_observe",
            more::a_stage_or_commit_grant_reads_no_staged_state_but_through_observe::<R>,
        ),
        Scenario::new(
            "a_channel_that_does_not_stage_creates_a_coordinate_once",
            more::a_channel_that_does_not_stage_creates_a_coordinate_once::<R>,
        ),
    ]
}

/// A coordinate is created once: a commit of the bytes it already holds is `Duplicate`, on a
/// channel that stages and on one that does not, and the coordinate is as it was.
fn a_second_commit_of_the_same_bytes_is_duplicate<R: PublisherRig>(rig: &R) {
    for shape in [ChannelShape::staging(), ChannelShape::without_staging()] {
        let mut p = rig.publisher(&shape);
        let set = Set::of(&["a"]);
        if shape.stages {
            stage(rig, &mut p, &set);
        }
        commit(rig, &mut p, &set);
        let blobs = set.blobs();
        let items = set.items(&blobs);
        let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
        same(
            must(p.commit(&grant, &items), "a second commit"),
            CommitAnswer::Duplicate,
            &format!("a second commit of the same bytes ({shape:?})"),
        );
        present_held(stranger(&mut p, &set.coords[0]));
        same(
            fetched(rig, &mut p, &set.coords[0]),
            Some(b"a".to_vec()),
            "the bytes after the second commit",
        );
    }
}

/// A stage over a coordinate the caller itself has staged is `Duplicate` too, with the same
/// bytes and with other bytes, and the staged bytes are the first ones.
fn a_stage_over_ones_own_staged_coordinate_is_duplicate<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    stage(rig, &mut p, &set);
    let mut other = Set::of(&["a"]);
    other.bytes[0] = b"other".to_vec();
    for (what, again) in [("the same bytes", &set), ("other bytes", &other)] {
        let blobs = again.blobs();
        let items = again.items(&blobs);
        let grant = rig.grant(&p, GrantPhase::Stage, &again.coords);
        same(
            must(p.stage(&grant, &items), "a second stage"),
            StageAnswer::Duplicate,
            &format!("a stage over one's own staged coordinate with {what}"),
        );
    }
    commit(rig, &mut p, &set);
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(b"a".to_vec()),
        "the bytes committed after the second stages",
    );
}

fn observe_absent_is_not_an_error<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    same(
        stranger(&mut p, &set.coords[0]),
        Presence::Absent,
        "a coordinate nobody wrote",
    );
    a_held_coordinate_does_not_answer_for_one_that_differs_in_a_field(rig);
    more::a_sibling_is_another_coordinate_to_stage_commit_and_observe(rig);
}

/// A coordinate is the whole of its five fields: the one held that differs from the one asked
/// for in the version, the platform or the channel is not it, to `observe` or to `fetch`.
fn a_held_coordinate_does_not_answer_for_one_that_differs_in_a_field<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let held = coordinate("a");
    publish(rig, &mut p, &Set::at(vec![held.clone()]));
    present_held(stranger(&mut p, &held));
    let others = [
        (
            "version",
            Coordinate {
                version: "1.2.4".to_owned(),
                ..held.clone()
            },
        ),
        (
            "platform",
            Coordinate {
                platform: "darwin-arm64".to_owned(),
                ..held.clone()
            },
        ),
        (
            "channel",
            Coordinate {
                channel: "beta".to_owned(),
                ..held.clone()
            },
        ),
    ];
    for (field, asked) in &others {
        same(
            stranger(&mut p, asked),
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

fn stage_then_observe_returns_staged_with_the_same_digest<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    stage(rig, &mut p, &set);
    let held = staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    same(
        held.digest,
        sha(b"a"),
        "the digest of the staged coordinate",
    );
}

fn a_staged_coordinate_is_not_readable_by_a_stranger<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    stage(rig, &mut p, &set);
    same(
        stranger(&mut p, &set.coords[0]),
        Presence::Absent,
        "a staged coordinate seen by a stranger",
    );
}

fn a_stage_grant_observes_the_staged_state_it_made<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    stage(rig, &mut p, &set);
    staged_held(observed_alone(rig, &mut p, GrantPhase::Stage, &set, 0));
}

fn a_commit_grant_observes_it_too<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    stage(rig, &mut p, &set);
    staged_held(observed_alone(rig, &mut p, GrantPhase::Commit, &set, 0));
}

fn commit_makes_the_staged_coordinates_readable_with_the_same_digest<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a", "b"]);
    publish(rig, &mut p, &set);
    for (at, coord) in set.coords.iter().enumerate() {
        let held = present_held(stranger(&mut p, coord));
        same(
            held.digest,
            sha(&set.bytes[at]),
            "the digest of a committed coordinate",
        );
        same(
            fetched(rig, &mut p, coord),
            Some(set.bytes[at].clone()),
            "its bytes",
        );
    }
}

fn stage_of_present_same_is_duplicate_and_changes_nothing<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    publish(rig, &mut p, &set);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Stage, &set.coords);
    same(
        must(p.stage(&grant, &items), "stage"),
        StageAnswer::Duplicate,
        "a stage over the same bytes",
    );
    present_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(b"a".to_vec()),
        "the bytes after it",
    );
}

fn commit_over_present_different_leaves_old_bytes<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::without_staging());
    let set = Set::of(&["a"]);
    rig.arrange(
        &mut p,
        &Arrangement::HeldByAnother {
            coordinate: set.coords[0].clone(),
            bytes: b"theirs".to_vec(),
        },
    );
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    same(
        must(p.commit(&grant, &items), "commit"),
        CommitAnswer::Duplicate,
        "a commit over other bytes",
    );
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(b"theirs".to_vec()),
        "the bytes after it",
    );
}

fn abort_gives_staged_state_back_and_never_touches_a_committed_coordinate<R: PublisherRig>(
    rig: &R,
) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let (a, b, c) = (Set::of(&["a"]), Set::of(&["b"]), Set::of(&["c"]));
    publish(rig, &mut p, &a);
    stage(rig, &mut p, &b);
    stage(rig, &mut p, &c);
    let both = Set::of(&["a", "b"]);
    let grant = rig.grant(&p, GrantPhase::Stage, &both.coords);
    must(p.abort(&grant, &both.coords), "abort");
    same(
        observed(rig, &mut p, GrantPhase::Stage, &b, 0),
        Presence::Absent,
        "the staged coordinate after the abort",
    );
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &c, 0));
    present_held(stranger(&mut p, &a.coords[0]));
    same(
        fetched(rig, &mut p, &a.coords[0]),
        Some(b"a".to_vec()),
        "the committed bytes after it",
    );
    more::an_abort_reaches_only_the_coordinates_it_names(rig);
}

fn a_set_commit_makes_every_coordinate_readable_or_none<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a", "b"]);
    publish(rig, &mut p, &set);
    for coord in &set.coords {
        present_held(stranger(&mut p, coord));
    }
    let mut failing = rig.publisher(&ChannelShape::staging());
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
}

fn a_channel_that_does_not_stage_commits_with_the_bytes<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::without_staging());
    same(
        p.stages(),
        false,
        "stages() of a channel that does not stage",
    );
    let set = Set::of(&["a"]);
    commit(rig, &mut p, &set);
    present_held(stranger(&mut p, &set.coords[0]));
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(b"a".to_vec()),
        "the bytes committed",
    );
}

fn no_digest_exposed_is_refused<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    rig.arrange(
        &mut p,
        &Arrangement::HeldByAnother {
            coordinate: set.coords[0].clone(),
            bytes: b"theirs".to_vec(),
        },
    );
    let held = present_held(stranger(&mut p, &set.coords[0]));
    same(
        held.digest,
        sha(b"theirs"),
        "the digest a coordinate of another writer is reported with",
    );
}

fn yank_keeps_bytes_fetchable<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    publish(rig, &mut p, &set);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    must(p.yank(&grant, &set.coords[0], "a reason"), "yank");
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(b"a".to_vec()),
        "the bytes of a yanked coordinate",
    );
    same(
        matches!(stranger(&mut p, &set.coords[0]), Presence::Yanked(_)),
        true,
        "a yanked coordinate is observed as Yanked",
    );
    more::a_yank_reaches_only_the_coordinate_it_names_and_needs_it_held(rig);
}

fn yanked_coordinate_cannot_be_committed_again<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let set = Set::of(&["a"]);
    publish(rig, &mut p, &set);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    must(p.yank(&grant, &set.coords[0], "a reason"), "yank");
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let staged = p.stage(&rig.grant(&p, GrantPhase::Stage, &set.coords), &items);
    same(
        staged,
        Ok(StageAnswer::Duplicate),
        "a stage over a yanked coordinate",
    );
    let again = p.commit(&grant, &items);
    same(
        matches!(again, Ok(CommitAnswer::Committed)),
        false,
        "a commit over a yanked coordinate",
    );
    same(
        matches!(stranger(&mut p, &set.coords[0]), Presence::Yanked(_)),
        true,
        "the coordinate after the second commit",
    );
}

fn lost_answer_is_resolvable_by_observe<R: PublisherRig>(rig: &R) {
    let set = Set::of(&["a"]);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let mut landed = rig.publisher(&ChannelShape::staging());
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
    present_held(stranger(&mut landed, &set.coords[0]));
    let mut not_landed = rig.publisher(&ChannelShape::staging());
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
    staged_held(observed(rig, &mut not_landed, GrantPhase::Commit, &set, 0));
}

fn list_may_lag_but_observe_is_exact<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    rig.arrange(&mut p, &Arrangement::ListLags);
    let set = Set::of(&["a"]);
    publish(rig, &mut p, &set);
    present_held(stranger(&mut p, &set.coords[0]));
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
    a_listing_names_only_what_is_held_in_its_scope(rig);
}

/// A listing may leave coordinates out, but what it names is held, in the channel and under
/// the prefix of its scope, and carries what the registry holds beside each.
fn a_listing_names_only_what_is_held_in_its_scope<R: PublisherRig>(rig: &R) {
    let mut p = rig.publisher(&ChannelShape::staging());
    let mut set = Set::at(vec![
        coordinate("a"),
        coordinate("ab"),
        coordinate("b"),
        Coordinate {
            channel: "beta".to_owned(),
            ..coordinate("a")
        },
    ]);
    set.held.exceptional = true;
    publish(rig, &mut p, &set);
    for (channel, prefix) in [("stable", "a"), ("stable", ""), ("beta", "a")] {
        let scope = ListScope {
            channel: channel.to_owned(),
            prefix: prefix.to_owned(),
        };
        let grant = rig.grant(&p, GrantPhase::Read, &set.coords);
        let listed = must(p.list(&grant, &scope), "list");
        for entry in listed {
            let c = &entry.coordinate;
            same(
                set.coords.contains(c) & (c.channel == channel) & c.name.starts_with(prefix),
                true,
                &format!("{c:?} listed for the channel {channel:?} and the prefix {prefix:?}"),
            );
            same(
                entry.held,
                Held {
                    digest: sha(c.name.as_bytes()),
                    ..set.held.clone()
                },
                &format!("what a listing holds beside {c:?}"),
            );
        }
    }
}
