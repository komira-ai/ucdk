// SPDX-License-Identifier: Apache-2.0
//! What a publisher kit asks of its implementation, and the helpers every scenario of it shares:
//! the channel shapes a rig declares, the worlds a scenario arranges, the coordinates, bytes and
//! siblings the rig supplies, and the sets a scenario publishes.
//!
//! A rig declares the shapes of channel it can build, and the kit builds only those: a scenario
//! that needs a channel the rig did not declare is skipped, with the reason, and a skip is never a
//! pass. A declaration is held both ways, because a rig that states a thing about itself could
//! otherwise hide a defect: the channel it builds for a shape must say what the shape says.
//!
//! A scenario about a promise a channel makes by what it can do, and not by who may read it, runs
//! on every declared shape that can do it: a lifecycle scenario (a race, a duplicate, a set, an
//! abort, a yank, `newest`, a stamp, a read-back) on each shape that stages or that commits, public
//! or private, addressed by coordinate or by group. The gate of such a scenario is the capability
//! (`stages`, `commit_unit`, `yank_unit`, `private`, `group_addressed`), and never that the shape
//! is plain. A reader of a private channel holds a read grant where a reader of a public one is a
//! stranger (`looked`), and staged state of a channel addressed by group is observed told the set
//! it was staged in.

use sha2::{Digest as _, Sha256};
use ucdk_core::Hash256;
use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::{ArtifactType, Coordinate, Stamped};

use super::{coordinate, fail, must, must_some, same};
use crate::{Blob, CommitAnswer, CommitUnit, Grant, Publisher, StageAnswer, StageItem};

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

    /// A public channel that stages and addresses staged state by the group of the declared set:
    /// reads need no grant, and an observe of staged state is told the set.
    pub const fn public_grouped() -> Self {
        Self {
            group_addressed: true,
            ..Self::staging()
        }
    }

    /// A private channel that stages and addresses staged state by the group of the declared set.
    pub const fn private_grouped() -> Self {
        Self {
            stages: true,
            private: true,
            group_addressed: true,
        }
    }
}

/// A day, in milliseconds: how far a scenario moves the clock to expire the grants it made.
pub(super) const A_DAY_MS: u64 = 86_400_000;

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
    /// coordinate. The commit then answers `Duplicate`, and only `Duplicate`: the staged state
    /// was not lost (that is [`Arrangement::LoseStagedState`]), the version was taken.
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
    /// What was staged is gone, and nobody holds the coordinate: the reservation was lost. The
    /// next commit of it is an error that promises no effect (`Refused` or `Permanent`), an
    /// observe finds `Absent`, and a new `stage` of it works.
    LoseStagedState,
    /// The commits that land from now on are readable to an observe only after it was asked this
    /// many times, so that an answer is `Absent` while the coordinate is becoming readable.
    CommitVisibleAfter(u32),
    /// `coordinate` was claimed by a group that failed, and can never be published: it is
    /// burned. Only a rig that can say [`PublisherRig::can_burn`] is asked for it.
    BurnedByFailedGroup { coordinate: Coordinate },
}

/// One read an observe made on a channel that addresses staged state by group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStep {
    Group,
    Index,
}

/// The name and the bytes of one member, as the channel takes them: a registry whose grammar
/// refuses the literal name or bytes of a scenario supplies ones it accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Where a sibling is published relative to the coordinate it is a sibling of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// In the same set as the base: the channel takes the two only together.
    SameSet,
    /// In a set of its own, beside the base that is held or staged.
    OwnSet,
}

/// A coordinate that differs from a base in one field, and how the channel takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sibling {
    /// The field it differs in: `channel`, `version`, `platform` or `type`.
    pub field: &'static str,
    pub coordinate: Coordinate,
    pub placement: Placement,
}

/// A coordinate of `base`'s name that differs from it in one field, with the field's name.
pub(super) fn differing_in_one_field(base: &Coordinate) -> Vec<(&'static str, Coordinate)> {
    vec![
        (
            "channel",
            Coordinate {
                channel: "beta".to_owned(),
                ..base.clone()
            },
        ),
        (
            "version",
            Coordinate {
                version: "1.2.4".to_owned(),
                ..base.clone()
            },
        ),
        (
            "platform",
            Coordinate {
                platform: "darwin-arm64".to_owned(),
                ..base.clone()
            },
        ),
        (
            "type",
            Coordinate {
                r#type: ArtifactType::Oci as i32,
                ..base.clone()
            },
        ),
    ]
}

/// The siblings of `base` that a rig which does not say otherwise has: one for each field, each
/// published in a set of its own. A rig that changes some starts from them.
pub fn every_field_in_a_set_of_its_own(base: &Coordinate) -> Vec<Sibling> {
    differing_in_one_field(base)
        .into_iter()
        .map(|(field, coordinate)| Sibling {
            field,
            coordinate,
            placement: Placement::OwnSet,
        })
        .collect()
}

/// What an implementation provides: the shapes of channel it builds, a channel of each, the
/// grants it accepts, the few changes to its world a scenario needs, and the coordinates and
/// bytes its grammar accepts. A rig that overrides none of the last hooks says that the literal
/// coordinates and bytes of the kit are fine.
///
/// The `Held` a scenario passes to `stage` and `commit` carries an empty digest, except in the
/// scenario about a registry that echoes the declared digest. The seam allows that: `stage` and
/// `commit` carry the `Held` so that a registry can store the stamp and the exceptional mark, and
/// the digest `observe` reports is the one the registry verified over the bytes it stored, so a
/// digest the writer declares is no precondition of either call.
pub trait PublisherRig {
    type Publisher: Publisher;

    /// The shapes of channel the rig builds. There is no default: a rig states it. A scenario
    /// that needs a shape not among them is skipped, and an empty list fails the kit.
    fn shapes(&self) -> Vec<ChannelShape>;

    /// A fresh, empty channel of `shape`, one of the declared shapes.
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

    /// The coordinate the kit calls `label`. Distinct labels map to distinct coordinates, and a
    /// label that starts with another maps to a name that starts with the other's name, because
    /// the kit lists by name prefix; the kit checks both.
    fn coordinate(&self, label: &str) -> Coordinate {
        coordinate(label)
    }

    /// The name and bytes of the member at `k`; two seeds give two valid, different members, the
    /// kit's own (seed 0) and another writer's (seed 1). The same seed gives the same member.
    fn payload(&self, k: &Coordinate, seed: u8) -> Payload {
        let mut bytes = k.name.clone().into_bytes();
        if seed != 0 {
            bytes.push(seed);
        }
        Payload {
            name: k.name.clone(),
            bytes,
        }
    }

    /// The coordinates of `base`'s name that differ from it in one field, which the channel
    /// holds, each with how it is published. The default is every field, each in a set of its
    /// own. A rig names at least one that the scenarios can publish beside the base (a later
    /// platform of a version the registry seals is not one): the scenarios about siblings are
    /// parts of scenarios about other things, so a rig that names none fails
    /// `the_declarations_of_a_rig_hold_of_what_it_builds`.
    fn siblings(&self, base: &Coordinate) -> Vec<Sibling> {
        every_field_in_a_set_of_its_own(base)
    }

    /// Coordinates the channel cannot hold at all (a platform it has no name for); every verb on
    /// one is `Refused` before anything is sent. The default is none.
    fn unrepresentable(&self) -> Vec<Coordinate> {
        Vec::new()
    }

    /// True if a version is sealed by the first group that wrote it, so that a later platform
    /// of the same name and version is `Refused(Channel)`. Held both ways: a rig that says so is
    /// held to `a_sealed_version_refuses_a_later_platform_and_observes_it_absent`, and one that
    /// says not is held to committing a platform beside a held one.
    fn seals_versions(&self) -> bool {
        false
    }

    /// True if the registry can burn a number, that is, if [`Arrangement::BurnedByFailedGroup`]
    /// can be made true of it. A registry with no failed groups cannot, and the scenario about a
    /// burned coordinate is skipped for it.
    fn can_burn(&self) -> bool {
        false
    }
}

// ---- the shapes of a rig ----

/// The declared shapes the scenarios of this part run on: public, and not addressed by group,
/// because a scenario observes by coordinate alone.
pub(super) fn plain_shapes<R: PublisherRig>(rig: &R) -> Vec<ChannelShape> {
    rig.shapes()
        .into_iter()
        .filter(|s| !s.private && !s.group_addressed)
        .collect()
}

/// The first declared plain shape that stages.
pub(super) fn staging_shape<R: PublisherRig>(rig: &R) -> Option<ChannelShape> {
    plain_shapes(rig).into_iter().find(|s| s.stages)
}

/// The first declared plain shape that does not stage.
pub(super) fn unstaged_shape<R: PublisherRig>(rig: &R) -> Option<ChannelShape> {
    plain_shapes(rig).into_iter().find(|s| !s.stages)
}

/// Runs `scenario` once on each declared plain shape, a fresh channel each: a promise a registry
/// makes of every channel is held on each shape the rig declares, and not on the first alone. A
/// rig with no plain shape fails the scenario, which the runner skips before it gets here.
pub(super) fn on_each_plain_shape<R: PublisherRig>(rig: &R, scenario: fn(&R, &ChannelShape)) {
    on_each_of(
        rig,
        &plain_shapes(rig),
        "a public channel to run on",
        scenario,
    );
}

/// Runs `scenario` once on each of `shapes`, a fresh channel each. A rig with none of them fails
/// the scenario, which the runner skips before it gets here.
pub(super) fn on_each_of<R: PublisherRig>(
    rig: &R,
    shapes: &[ChannelShape],
    what: &str,
    scenario: fn(&R, &ChannelShape),
) {
    must_some(shapes.first(), what);
    for shape in shapes {
        scenario(rig, shape);
    }
}

/// The declared shapes whose reads need no grant.
pub(super) fn public_shapes<R: PublisherRig>(rig: &R) -> Vec<ChannelShape> {
    rig.shapes().into_iter().filter(|s| !s.private).collect()
}

/// The declared shapes whose every read, a stranger's included, needs a grant.
pub(super) fn private_shapes<R: PublisherRig>(rig: &R) -> Vec<ChannelShape> {
    rig.shapes().into_iter().filter(|s| s.private).collect()
}

/// The declared shapes that stage and address the staged state by the group of the declared set.
pub(super) fn grouped_shapes<R: PublisherRig>(rig: &R) -> Vec<ChannelShape> {
    rig.shapes()
        .into_iter()
        .filter(|s| s.group_addressed && s.stages)
        .collect()
}

/// The phase of the grant an action holds first: the stage grant when the channel stages, and
/// the commit grant when it does not.
pub(super) fn first_phase<P: Publisher>(p: &P) -> GrantPhase {
    if p.stages() {
        GrantPhase::Stage
    } else {
        GrantPhase::Commit
    }
}

/// The declared shapes that stage, of any address and any privacy.
pub(super) fn staging_shapes<R: PublisherRig>(rig: &R) -> Vec<ChannelShape> {
    rig.shapes().into_iter().filter(|s| s.stages).collect()
}

/// The declared shapes that do not stage, public or private.
pub(super) fn unstaged_shapes<R: PublisherRig>(rig: &R) -> Vec<ChannelShape> {
    rig.shapes().into_iter().filter(|s| !s.stages).collect()
}

/// Runs `scenario` once on each declared shape, a fresh channel each, whatever the shape stages,
/// addresses or hides: a promise a scenario holds of every channel is held of each of them.
pub(super) fn on_each_shape<R: PublisherRig>(rig: &R, scenario: fn(&R, &ChannelShape)) {
    on_each_of(rig, &rig.shapes(), "a channel to run on", scenario);
}

/// Runs `scenario` once on each declared shape that stages.
pub(super) fn on_each_staging_shape<R: PublisherRig>(rig: &R, scenario: fn(&R, &ChannelShape)) {
    on_each_of(
        rig,
        &staging_shapes(rig),
        "a channel that stages to run on",
        scenario,
    );
}

/// Runs `scenario` once on each declared shape that does not stage.
pub(super) fn on_each_unstaged_shape<R: PublisherRig>(rig: &R, scenario: fn(&R, &ChannelShape)) {
    on_each_of(
        rig,
        &unstaged_shapes(rig),
        "a channel that does not stage to run on",
        scenario,
    );
}

/// The phases of the grants an action of a channel of `shape` holds, each of which may observe:
/// the read grant, the stage grant of a channel that stages, and the commit grant.
pub(super) fn phases_of(shape: &ChannelShape) -> Vec<GrantPhase> {
    let mut all = vec![GrantPhase::Read];
    if shape.stages {
        all.push(GrantPhase::Stage);
    }
    all.push(GrantPhase::Commit);
    all
}

/// A fresh channel of `shape`, held to what the shape says.
pub(super) fn built<R: PublisherRig>(rig: &R, shape: &ChannelShape) -> R::Publisher {
    let p = rig.publisher(shape);
    same(
        p.stages(),
        shape.stages,
        &format!("stages() of the channel built for {shape:?}"),
    );
    p
}

/// A fresh channel of the first declared plain shape that stages.
pub(super) fn staging_publisher<R: PublisherRig>(rig: &R) -> R::Publisher {
    built(
        rig,
        &must_some(staging_shape(rig), "a public channel that stages"),
    )
}

/// A fresh channel of the first declared plain shape that does not stage.
pub(super) fn unstaged_publisher<R: PublisherRig>(rig: &R) -> R::Publisher {
    built(
        rig,
        &must_some(unstaged_shape(rig), "a public channel that does not stage"),
    )
}

/// The gate of a scenario that runs on any plain channel.
pub(super) fn needs_a_plain_channel<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    plain_shapes(rig)
        .is_empty()
        .then_some("the rig declares no public channel without a group address")
}

/// The gate of a scenario about staged state.
pub(super) fn needs_staging<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    staging_shape(rig)
        .is_none()
        .then_some("the rig declares no public channel that stages")
}

/// The gate of a scenario about a channel whose commit carries the bytes.
pub(super) fn needs_no_staging<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    unstaged_shape(rig)
        .is_none()
        .then_some("the rig declares no public channel that does not stage")
}

/// The gate of a scenario about a conditional put on a channel whose commit carries the bytes.
pub(super) fn needs_a_conditional_commit<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    let Some(shape) = unstaged_shape(rig) else {
        return Some("the rig declares no public channel that does not stage");
    };
    (!built(rig, &shape).conditional_put())
        .then_some("the channel is not a conditional put: its commit may replace what is there")
}

/// The gate of a scenario that commits several coordinates in one call.
pub(super) fn needs_a_set_commit<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    (!plain_shapes(rig)
        .iter()
        .any(|s| built(rig, s).commit_unit() == CommitUnit::Set))
    .then_some("the rig declares no public channel that commits a set at once")
}

/// The gate of a scenario about staged state that holds on a channel of any shape that stages,
/// public or private, addressed by coordinate or by group.
pub(super) fn needs_a_staging_channel<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    staging_shapes(rig)
        .is_empty()
        .then_some("the rig declares no channel that stages")
}

/// The gate of a scenario about a channel whose commit carries the bytes, public or private.
pub(super) fn needs_an_unstaged_channel<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    unstaged_shapes(rig)
        .is_empty()
        .then_some("the rig declares no channel that does not stage")
}

/// The gate of a scenario about grants that holds on a channel of any shape.
pub(super) fn needs_a_channel<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    rig.shapes()
        .is_empty()
        .then_some("the rig declares no channel")
}

/// The gate of a scenario about what a stranger reads of a public channel.
pub(super) fn needs_a_public_channel<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    public_shapes(rig)
        .is_empty()
        .then_some("the rig declares no public channel")
}

/// The gate of a scenario about a channel whose every read needs a grant.
pub(super) fn needs_a_private_channel<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    private_shapes(rig)
        .is_empty()
        .then_some("the rig declares no private channel")
}

/// The gate of a scenario about staged state addressed by group.
pub(super) fn needs_a_grouped_channel<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    grouped_shapes(rig)
        .is_empty()
        .then_some("the rig declares no channel that stages and is addressed by group")
}

// ---- sets ----

/// The coordinates, members and `Held` of a set a scenario publishes.
#[derive(Clone)]
pub(super) struct Set {
    pub(super) coords: Vec<Coordinate>,
    pub(super) payloads: Vec<Payload>,
    pub(super) held: Held,
}

impl Set {
    /// One coordinate per label, each with the rig's own member.
    pub(super) fn of<R: PublisherRig>(rig: &R, labels: &[&str]) -> Self {
        Self::at(rig, labels.iter().map(|l| rig.coordinate(l)).collect())
    }

    /// The given coordinates, each with the rig's own member.
    pub(super) fn at<R: PublisherRig>(rig: &R, coords: Vec<Coordinate>) -> Self {
        Self {
            payloads: coords.iter().map(|c| rig.payload(c, 0)).collect(),
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

    /// The same set with the member of every coordinate as another writer would have made it.
    pub(super) fn reseeded<R: PublisherRig>(&self, rig: &R, seed: u8) -> Self {
        Self {
            coords: self.coords.clone(),
            payloads: self.coords.iter().map(|c| rig.payload(c, seed)).collect(),
            held: self.held.clone(),
        }
    }

    /// The bytes of the member at `at`.
    pub(super) fn bytes(&self, at: usize) -> &[u8] {
        &self.payloads[at].bytes
    }

    /// The identity digest of the member at `at`, from the bytes the kit sent.
    pub(super) fn digest(&self, at: usize) -> String {
        sha(self.bytes(at))
    }

    pub(super) fn blobs(&self) -> Vec<Blob<'_>> {
        self.payloads
            .iter()
            .map(|p| Blob {
                name: p.name.as_str(),
                bytes: &p.bytes,
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

/// The sets to publish, in order, to hold `base` and `sibling` in a channel: one set of both when
/// the channel takes them only together, and otherwise one set each.
pub(super) fn sets_beside<R: PublisherRig>(
    rig: &R,
    base: &Coordinate,
    sibling: &Sibling,
) -> Vec<Set> {
    match sibling.placement {
        Placement::SameSet => vec![Set::at(rig, vec![base.clone(), sibling.coordinate.clone()])],
        Placement::OwnSet => vec![
            Set::at(rig, vec![base.clone()]),
            Set::at(rig, vec![sibling.coordinate.clone()]),
        ],
    }
}

/// The siblings of `base` a scenario may publish beside it: a registry that seals a version by
/// its first group takes no later platform of it, and that is held by its own scenario.
pub(super) fn siblings_beside<R: PublisherRig>(rig: &R, base: &Coordinate) -> Vec<Sibling> {
    rig.siblings(base)
        .into_iter()
        .filter(|s| {
            !(rig.seals_versions() && s.field == "platform" && s.placement == Placement::OwnSet)
        })
        .collect()
}

/// The arrangement that makes `k` held by another writer, with that writer's own member.
pub(super) fn held_by_another<R: PublisherRig>(rig: &R, k: &Coordinate) -> Arrangement {
    Arrangement::HeldByAnother {
        coordinate: k.clone(),
        bytes: rig.payload(k, 1).bytes,
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

pub(super) fn is_yanked(presence: &Presence) -> bool {
    matches!(presence, Presence::Yanked(_))
}

/// Stages `set` with a stage grant. A channel that does not stage stages nothing: its `stage` is
/// never called.
pub(super) fn stage<R: PublisherRig>(rig: &R, p: &mut R::Publisher, set: &Set) {
    if !p.stages() {
        return;
    }
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(p, GrantPhase::Stage, &set.coords);
    same(
        must(p.stage(&grant, &items), "stage"),
        StageAnswer::Staged,
        "the answer to a stage",
    );
}

/// Commits `set` with a commit grant, staged or not: in one call when the channel commits a set
/// at once, and a coordinate to a call when it commits each on its own.
pub(super) fn commit<R: PublisherRig>(rig: &R, p: &mut R::Publisher, set: &Set) {
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let calls: Vec<&[StageItem<'_>]> = if p.commit_unit() == CommitUnit::Set {
        vec![&items[..]]
    } else {
        items.chunks(1).collect()
    };
    for call in calls {
        let coords: Vec<Coordinate> = call.iter().map(|i| i.coordinate.clone()).collect();
        let grant = rig.grant(p, GrantPhase::Commit, &coords);
        same(
            must(p.commit(&grant, call), "commit"),
            CommitAnswer::Committed,
            "the answer to a commit",
        );
    }
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

/// The bytes a stranger fetches for `k`, as the read-back of a publish on a public channel does.
pub(super) fn fetched_by_a_stranger<P: Publisher>(p: &mut P, k: &Coordinate) -> Option<Vec<u8>> {
    must(p.fetch(&Grant::none(), k), "a fetch by a stranger")
}

/// What a reader sees of `k` on a channel of `shape`: a stranger on a public channel, which needs
/// no grant, and the holder of a read grant on a private one, which turns a stranger away.
pub(super) fn looked<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
    p: &mut R::Publisher,
    k: &Coordinate,
) -> Presence {
    if shape.private {
        let grant = rig.grant(p, GrantPhase::Read, std::slice::from_ref(k));
        must(p.observe(&grant, k, &[]), "an observe with a read grant")
    } else {
        stranger(p, k)
    }
}

/// The bytes a reader fetches for `k` on a channel of `shape`, as the read-back of a publish
/// does: as a stranger on a public channel, and with a read grant on a private one.
pub(super) fn fetched_by_a_reader<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
    p: &mut R::Publisher,
    k: &Coordinate,
) -> Option<Vec<u8>> {
    if shape.private {
        fetched(rig, p, k)
    } else {
        fetched_by_a_stranger(p, k)
    }
}

/// The set of `sets` that holds `k`, and the position of `k` in it: the set `k` was staged in,
/// which a channel addressed by group finds staged state by.
pub(super) fn holder_of<'a>(sets: &'a [Set], k: &Coordinate) -> (&'a Set, usize) {
    let found = sets
        .iter()
        .find_map(|set| set.coords.iter().position(|c| c == k).map(|at| (set, at)));
    must_some(found, "the set the coordinate was staged in")
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
