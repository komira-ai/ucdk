// SPDX-License-Identifier: Apache-2.0
//! The publisher rig: the fake registry in each shape, the grants its issuer gives, the
//! declarations a rig makes about itself, and the stubs that break one promise of a registry.

use std::time::Duration;

use ucdk_core::{Clock, ManualClock};
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::{Held, Presence};
use ucdk_release_fake::{FakeRegistry, Read, coordinate_key};
use ucdk_release_proto::v1::{ArtifactType, Coordinate, Reason, Repository};
use ucdk_release_seams::kit::{
    Arrangement, ChannelShape, LOCATION, Payload, Placement, PublisherRig, ReadStep, Sibling,
    coordinate, every_field_in_a_set_of_its_own, federated_repository,
};
use ucdk_release_seams::{
    CommitAnswer, CommitUnit, Grant, ListScope, Listed, Publisher, SeamError, StageAnswer,
    StageItem, YankUnit,
};

use super::publisher_grant::{issue, scope_of};
use super::secret::texts;

pub use super::publisher_flaw::{Flaw, Hook};

/// The platform of the coordinate the conforming rig's channel cannot hold.
const FOREIGN_PLATFORM: &str = "plan9-mips";

/// A rig of the fake registry. It declares the shapes it builds and says of its grammar what is
/// true of it; a builder changes a declaration, or the registry behind it, one way at a time.
pub struct FakePublisherRig {
    clock: ManualClock,
    flaw: Option<Flaw>,
    /// The stub is behind the channels that do not stage only.
    flaw_only_unstaged: bool,
    hook: Option<Hook>,
    shapes: Vec<ChannelShape>,
    /// The rig says a version is sealed by its first group.
    declares_seals: bool,
    /// The registry seals a version by its first group.
    seals: bool,
    per_coordinate: bool,
    unconditional: bool,
    version_yank: bool,
    narrow: bool,
    burns: bool,
    /// The channel holds every coordinate: nothing is unrepresentable.
    holds_all: bool,
}

impl FakePublisherRig {
    /// The fake in every shape the kits use, with literal coordinates and bytes.
    pub fn conforming() -> Self {
        Self {
            clock: ManualClock::at(1_000_000),
            flaw: None,
            flaw_only_unstaged: false,
            hook: None,
            shapes: vec![
                ChannelShape::staging(),
                ChannelShape::without_staging(),
                ChannelShape {
                    stages: true,
                    private: true,
                    group_addressed: true,
                },
                ChannelShape {
                    stages: true,
                    private: true,
                    group_addressed: false,
                },
                ChannelShape {
                    stages: false,
                    private: true,
                    group_addressed: false,
                },
            ],
            declares_seals: false,
            seals: false,
            per_coordinate: false,
            unconditional: false,
            version_yank: false,
            narrow: false,
            burns: true,
            holds_all: false,
        }
    }

    /// A channel with no grammar to refuse by: it holds every coordinate.
    pub fn holding_every_coordinate() -> Self {
        Self {
            holds_all: true,
            ..Self::conforming()
        }
    }

    pub fn flawed(flaw: Flaw) -> Self {
        Self::conforming().with_flaw(flaw)
    }

    /// The same rig with the stub of `flaw` behind it.
    #[must_use]
    pub fn with_flaw(self, flaw: Flaw) -> Self {
        let version_yank = self.version_yank
            || matches!(
                flaw,
                Flaw::VersionYankIsPerCoordinate | Flaw::VersionYankIgnoresTheGrant
            );
        Self {
            flaw: Some(flaw),
            version_yank,
            ..self
        }
    }

    /// The same rig with the stub of `flaw` behind the channels that do not stage only: the
    /// channels of the other shapes are the conforming fake.
    #[must_use]
    pub fn with_flaw_only_unstaged(self, flaw: Flaw) -> Self {
        Self {
            flaw_only_unstaged: true,
            ..self.with_flaw(flaw)
        }
    }

    /// The same rig making a wrong declaration of its coordinates, bytes or siblings.
    #[must_use]
    pub fn hooked(self, hook: Hook) -> Self {
        Self {
            hook: Some(hook),
            ..self
        }
    }

    /// The same rig declaring only `shapes`.
    #[must_use]
    pub fn with_shapes(self, shapes: Vec<ChannelShape>) -> Self {
        Self { shapes, ..self }
    }

    /// A rig that builds one public channel that does not stage, one call to a commit.
    pub fn restricted() -> Self {
        Self::conforming().with_shapes(vec![ChannelShape::without_staging()])
    }

    /// A registry that seals a version by its first group, and a rig that says so.
    pub fn sealing() -> Self {
        Self {
            declares_seals: true,
            seals: true,
            ..Self::conforming()
        }
    }

    /// A rig that says the registry seals a version, which it does not.
    pub fn declaring_seals_only() -> Self {
        Self {
            declares_seals: true,
            ..Self::conforming()
        }
    }

    /// A registry that seals a version, which the rig does not say.
    pub fn sealing_undeclared() -> Self {
        Self {
            seals: true,
            ..Self::conforming()
        }
    }

    /// A registry whose commit may replace what is there, and a rig that says so.
    pub fn unconditional() -> Self {
        Self {
            unconditional: true,
            ..Self::conforming()
        }
    }

    /// A registry that commits each coordinate on its own.
    pub fn per_coordinate() -> Self {
        Self {
            per_coordinate: true,
            ..Self::conforming()
        }
    }

    /// A registry that yanks every platform of a version at one call.
    pub fn yanking_a_version() -> Self {
        Self {
            version_yank: true,
            ..Self::conforming()
        }
    }

    /// A registry with no failed groups: it cannot burn a number.
    pub fn unable_to_burn() -> Self {
        Self {
            burns: false,
            ..Self::conforming()
        }
    }

    /// A registry with a grammar of its own, as a real one has: names of two parts, one
    /// platform spelling, a member name that is a file name, one platform that is published with
    /// the base, and a version that is sealed by the group that first wrote it.
    pub fn narrow() -> Self {
        Self {
            narrow: true,
            declares_seals: true,
            seals: true,
            ..Self::conforming()
        }
    }

    pub(super) fn issue(
        &self,
        repository: &Repository,
        phase: GrantPhase,
        coordinates: &[Coordinate],
    ) -> Grant {
        issue(&self.clock, repository, phase, coordinates)
    }

    /// The coordinate the registry cannot hold, as it is really.
    fn banned(&self) -> Vec<Coordinate> {
        if self.holds_all {
            return Vec::new();
        }
        vec![Coordinate {
            platform: if self.narrow {
                "win-32".to_owned()
            } else {
                FOREIGN_PLATFORM.to_owned()
            },
            ..self.coordinate("a")
        }]
    }

    fn is(&self, hook: Hook) -> bool {
        self.hook == Some(hook)
    }
}

pub struct Flawed {
    pub(super) inner: FakeRegistry,
    pub(super) clock: ManualClock,
    /// Every coordinate that was in a `stage` or `commit` call, with its bytes.
    known: Vec<(Coordinate, Vec<u8>)>,
    /// The digest each coordinate was declared with, when it was declared one.
    declared: Vec<(Coordinate, String)>,
    banned: Vec<Coordinate>,
    flaw: Option<Flaw>,
    shape: ChannelShape,
    seals: bool,
    version_yank: bool,
    calls: u32,
    commit_calls: u32,
    lost: bool,
    /// A commit was arranged to lose a race.
    raced: bool,
    /// The staged state was arranged to vanish.
    vanished: bool,
    /// A commit was arranged to become readable late.
    becoming: bool,
    /// The coordinates that were arranged to be burned.
    burned: Vec<Coordinate>,
    list_lags: bool,
}

impl PublisherRig for FakePublisherRig {
    type Publisher = Flawed;

    fn shapes(&self) -> Vec<ChannelShape> {
        self.shapes.clone()
    }

    fn publisher(&self, shape: &ChannelShape) -> Flawed {
        let flaw = self
            .flaw
            .filter(|_| !(self.flaw_only_unstaged && shape.stages));
        let mut registry = FakeRegistry::new(&self.clock, LOCATION);
        if !shape.stages {
            registry = registry.without_staging();
        }
        if shape.private {
            registry = registry.private();
        }
        if shape.group_addressed {
            registry = registry.group_addressed();
        }
        if self.per_coordinate || flaw == Some(Flaw::CommitUnitSaysSet) {
            registry = registry.commit_per_coordinate();
        }
        if self.unconditional {
            registry = registry.tag_replacing();
        }
        Flawed {
            inner: registry,
            clock: self.clock.clone(),
            known: Vec::new(),
            declared: Vec::new(),
            banned: self.banned(),
            flaw,
            shape: *shape,
            seals: self.seals,
            version_yank: self.version_yank,
            calls: 0,
            commit_calls: 0,
            lost: false,
            raced: false,
            vanished: false,
            becoming: false,
            burned: Vec::new(),
            list_lags: false,
        }
    }

    fn arrange(&self, p: &mut Flawed, what: &Arrangement) {
        let next_commit = p.commit_calls + 1;
        match what {
            Arrangement::HeldByAnother { coordinate, bytes } => {
                p.inner.present_different(coordinate, bytes);
            }
            Arrangement::StagedByAnother { coordinate, bytes } => {
                p.inner.stage_other_bytes(coordinate, bytes);
            }
            Arrangement::RaceNextCommit(bytes) => {
                p.raced = true;
                p.inner.race_next_commit(bytes);
            }
            Arrangement::LoseAnswerAfterCommit => p.inner.lose_answer_after_commit(next_commit),
            Arrangement::LoseAnswerBeforeCommit => p.inner.lose_answer_before_commit(next_commit),
            Arrangement::FailAfterCommit => p.inner.fail_after_commit(next_commit),
            Arrangement::CommitSetFailsWhole => {
                if p.flaw == Some(Flaw::SetCommitIsTorn) && p.shape.stages {
                    p.inner.torn_commit();
                } else {
                    p.inner.commit_set_fails_whole();
                }
            }
            Arrangement::ListLags => {
                p.list_lags = true;
                p.inner.list_lag();
            }
            Arrangement::FailGroupRead => p.inner.fail_group_read(),
            Arrangement::FailIndexRead => p.inner.fail_index_read(),
            Arrangement::LoseStagedState => {
                p.vanished = true;
                let staged = p.known_where(|me, c| me.inner.is_staged(c));
                let grant = issue(
                    &p.clock,
                    &federated_repository(),
                    GrantPhase::Stage,
                    &staged,
                );
                p.inner
                    .abort(&grant, &staged)
                    .expect("the registry gives back what it staged");
            }
            Arrangement::CommitVisibleAfter(n) => {
                p.becoming = true;
                p.inner.read_lag(*n);
            }
            Arrangement::BurnedByFailedGroup { coordinate } => {
                p.burned.push(coordinate.clone());
                p.inner.burn(coordinate);
            }
        }
    }

    fn grant(&self, _p: &Flawed, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        self.issue(&federated_repository(), phase, coordinates)
    }

    fn foreign_grant(&self, _p: &Flawed, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        let elsewhere = Repository {
            location: "registry/other".to_owned(),
            r#type: ArtifactType::File as i32,
            ..federated_repository()
        };
        self.issue(&elsewhere, phase, coordinates)
    }

    fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
    }

    fn reads(&self, p: &Flawed) -> Vec<ReadStep> {
        p.inner
            .read_log()
            .iter()
            .map(|read| match read {
                Read::Group => ReadStep::Group,
                Read::Index => ReadStep::Index,
            })
            .collect()
    }

    fn coordinate(&self, label: &str) -> Coordinate {
        if self.narrow {
            Coordinate {
                name: format!("{label}_pkg"),
                platform: "linux-64".to_owned(),
                ..coordinate(label)
            }
        } else if self.is(Hook::CoordinateDropsThePrefix) {
            coordinate(&format!("x{label}"))
        } else {
            coordinate(label)
        }
    }

    fn payload(&self, k: &Coordinate, seed: u8) -> Payload {
        let seed = if self.is(Hook::PayloadIgnoresTheSeed) {
            0
        } else {
            seed
        };
        let mut bytes = if self.is(Hook::PayloadIsEmpty) {
            Vec::new()
        } else {
            format!("{}-{}-{}", k.name, k.version, k.platform).into_bytes()
        };
        if seed != 0 && !bytes.is_empty() {
            bytes.push(seed);
        }
        let name = if self.narrow {
            format!("mojo-{}-{}-0.conda", k.name, k.version)
        } else {
            k.name.clone()
        };
        Payload { name, bytes }
    }

    fn siblings(&self, base: &Coordinate) -> Vec<Sibling> {
        let platform = Coordinate {
            platform: "osx-arm64".to_owned(),
            ..base.clone()
        };
        let version = Coordinate {
            version: "1.2.4".to_owned(),
            ..base.clone()
        };
        if self.narrow {
            return vec![
                Sibling {
                    field: "platform",
                    coordinate: platform,
                    placement: Placement::SameSet,
                },
                Sibling {
                    field: "version",
                    coordinate: version,
                    placement: Placement::OwnSet,
                },
            ];
        }
        let mut all = every_field_in_a_set_of_its_own(base);
        match self.hook {
            Some(Hook::SiblingDiffersInTwoFields) => {
                all[0].coordinate = Coordinate {
                    version: "1.2.4".to_owned(),
                    ..all[0].coordinate.clone()
                };
            }
            Some(Hook::SiblingIsTheBase) => all[0].coordinate = base.clone(),
            _ => {}
        }
        all
    }

    fn unrepresentable(&self) -> Vec<Coordinate> {
        if self.is(Hook::UnrepresentableIsTheBase) {
            vec![self.coordinate("a")]
        } else {
            self.banned()
        }
    }

    fn seals_versions(&self) -> bool {
        self.declares_seals
    }

    fn can_burn(&self) -> bool {
        self.burns
    }
}

impl Flawed {
    fn is(&self, flaw: Flaw) -> bool {
        self.flaw == Some(flaw)
    }

    /// A grant the registry issues itself, for a stub that lets a caller in without the grant
    /// the call needs.
    fn internal(&self, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        issue(&self.clock, &federated_repository(), phase, coordinates)
    }

    /// Whether one of `ks` is a coordinate the channel cannot hold.
    fn holds_none_of(&self, ks: &[Coordinate]) -> bool {
        ks.iter().any(|k| self.banned.contains(k))
    }

    /// Whether a call over `ks` meets a version that was sealed by an earlier group: a held
    /// coordinate of the same channel, type, name and version, of another platform.
    fn sealed(&self, ks: &[Coordinate]) -> bool {
        self.seals
            && ks.iter().any(|k| {
                self.known.iter().any(|(held, _)| {
                    held.channel == k.channel
                        && held.r#type == k.r#type
                        && held.name == k.name
                        && held.version == k.version
                        && held.platform != k.platform
                        && !ks.contains(held)
                        && self.inner.bytes(held).is_some()
                })
            })
    }

    /// The answer a stub of a sealed version gives.
    fn answer_to_a_sealed_version<T>(&self, duplicate: T) -> Result<T, SeamError> {
        if self.is(Flaw::SealedVersionIsDuplicate) {
            Ok(duplicate)
        } else {
            Err(SeamError::Refused(Reason::Channel))
        }
    }

    /// `held` with a wrong digest when `flaw` is the flaw of the stub.
    fn misreported(&self, held: Held, flaw: Flaw) -> Held {
        if self.is(flaw) {
            digest_of(held, "bad")
        } else {
            held
        }
    }

    /// A coordinate a call once named that has the name of `k` and differs from it only in the
    /// field `flaw` ignores.
    fn sibling(&self, k: &Coordinate) -> Option<Coordinate> {
        let (version, platform, channel) = (
            matches!(
                self.flaw,
                Some(Flaw::ObserveIgnoresVersion | Flaw::FetchIgnoresVersion)
            ),
            matches!(
                self.flaw,
                Some(Flaw::ObserveIgnoresPlatform | Flaw::FetchIgnoresPlatform)
            ),
            matches!(
                self.flaw,
                Some(Flaw::ObserveIgnoresChannel | Flaw::FetchIgnoresChannel)
            ),
        );
        self.known
            .iter()
            .map(|(c, _)| c)
            .find(|c| {
                c.name == k.name
                    && *c != k
                    && (c.version == k.version) != version
                    && (c.platform == k.platform) != platform
                    && (c.channel == k.channel) != channel
            })
            .cloned()
    }

    fn remember(&mut self, items: &[StageItem<'_>]) {
        for item in items {
            self.known
                .push((item.coordinate.clone(), item.file.bytes.to_vec()));
            if !item.held.digest.is_empty() {
                self.declared
                    .push((item.coordinate.clone(), item.held.digest.clone()));
            }
        }
    }

    /// The coordinates a call once named that satisfy `keep`, each once.
    fn known_where(&self, keep: impl Fn(&Self, &Coordinate) -> bool) -> Vec<Coordinate> {
        let mut found: Vec<Coordinate> = Vec::new();
        for (coordinate, _) in &self.known {
            if keep(self, coordinate) && !found.contains(coordinate) {
                found.push(coordinate.clone());
            }
        }
        found
    }

    /// What the registry holds for `k`, staged or committed, as it sees it itself.
    pub(super) fn held_now(&mut self, k: &Coordinate) -> Option<Held> {
        let grant = self.internal(GrantPhase::Stage, std::slice::from_ref(k));
        match self.inner.observe(&grant, k, &[]) {
            Ok(Presence::Present(held) | Presence::Yanked(held) | Presence::Staged(held)) => {
                Some(held)
            }
            _ => None,
        }
    }

    /// Whether `g` is a grant of `scope` for this repository that has not expired and names
    /// every coordinate of `ks`.
    fn is_good_for(&self, g: &Grant, scope: &str, ks: &[Coordinate]) -> bool {
        let all = texts(g.secret());
        let fresh = g.expires_ms().is_none_or(|e| self.clock.now_millis() < e);
        let named = all.get(2..).unwrap_or_default();
        all.first().map(String::as_str) == Some(scope)
            && all.get(1).map(String::as_str) == Some(LOCATION)
            && fresh
            && ks.iter().all(|k| named.contains(&coordinate_key(k)))
    }

    /// The bytes the caller staged at `k` that are still staged, as the stub saw them.
    fn staged_bytes(&self, k: &Coordinate) -> Option<Vec<u8>> {
        if !self.inner.is_staged(k) {
            return None;
        }
        let latest = self.known.iter().rev().find(|(c, _)| c == k);
        latest.map(|(_, bytes)| bytes.clone())
    }

    /// The held coordinates that agree with `k` in channel, type, name and version.
    fn held_of_the_version(&self, k: &Coordinate) -> Vec<Coordinate> {
        self.known_where(|me, c| {
            me.inner.bytes(c).is_some()
                && c.channel == k.channel
                && c.r#type == k.r#type
                && c.name == k.name
                && c.version == k.version
        })
    }
}

fn digest_of(held: Held, digest: &str) -> Held {
    Held {
        digest: digest.to_owned(),
        ..held
    }
}

impl Publisher for Flawed {
    fn types(&self) -> &[ArtifactType] {
        self.inner.types()
    }

    fn stages(&self) -> bool {
        self.inner.stages() || (self.is(Flaw::StagesLies) && !self.shape.stages)
    }

    fn commit_unit(&self) -> CommitUnit {
        match self.flaw {
            Some(Flaw::CommitUnitSaysSet) => CommitUnit::Set,
            Some(Flaw::CommitUnitSaysCoordinate) => CommitUnit::Coordinate,
            _ => self.inner.commit_unit(),
        }
    }

    fn conditional_put(&self) -> bool {
        self.inner.conditional_put()
    }

    fn grant_limit(&self) -> usize {
        if self.is(Flaw::GrantLimitIsZero) {
            0
        } else {
            self.inner.grant_limit()
        }
    }

    fn yank_unit(&self) -> YankUnit {
        if self.version_yank {
            YankUnit::Version
        } else {
            YankUnit::Coordinate
        }
    }

    fn observe(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        set: &[StageItem<'_>],
    ) -> Result<Presence, SeamError> {
        self.calls += 1;
        let first = self.calls == 1;
        if self.holds_none_of(std::slice::from_ref(k)) {
            return if self.is(Flaw::UnrepresentableObserveIsAbsent) {
                Ok(Presence::Absent)
            } else {
                Err(SeamError::Refused(Reason::Channel))
            };
        }
        let staged = self.inner.is_staged(k);
        let scope = scope_of(g);
        let result = if self.is(Flaw::ObserveReadsTheLaggingList) && self.list_lags {
            Ok(Presence::Absent)
        } else {
            self.inner.observe(g, k, set)
        };
        let granted = !g.is_none();
        match (self.flaw, result) {
            (
                Some(
                    Flaw::ObserveIgnoresVersion
                    | Flaw::ObserveIgnoresPlatform
                    | Flaw::ObserveIgnoresChannel,
                ),
                Ok(Presence::Absent),
            ) => match self.sibling(k) {
                Some(sibling) => self.inner.observe(&Grant::none(), &sibling, &[]),
                None => Ok(Presence::Absent),
            },
            (Some(Flaw::AbsentIsAnError), Ok(Presence::Absent)) if first && !granted => {
                Err(SeamError::Permanent("not found".to_owned()))
            }
            (Some(Flaw::StagedDigestIsWrong), Ok(Presence::Staged(held))) => {
                Ok(Presence::Staged(digest_of(held, "bad")))
            }
            (Some(Flaw::StrangerSeesStaged), Ok(Presence::Absent)) if !granted && staged => {
                Ok(Presence::Staged(Held::default()))
            }
            (Some(Flaw::StageGrantIsBlindToStaged), Ok(Presence::Staged(_)))
                if scope == "stage" && set.is_empty() =>
            {
                Ok(Presence::Absent)
            }
            (Some(Flaw::CommitGrantIsBlindToStaged), Ok(Presence::Staged(_)))
                if scope == "commit" && set.is_empty() =>
            {
                Ok(Presence::Absent)
            }
            (Some(Flaw::PresentDigestIsWrong), Ok(Presence::Present(held)))
                if held.stamp.is_some() =>
            {
                Ok(Presence::Present(digest_of(held, "bad")))
            }
            (Some(Flaw::OthersHaveNoDigest), Ok(Presence::Present(held)))
                if held.stamp.is_none() =>
            {
                Ok(Presence::Present(digest_of(held, "")))
            }
            (Some(Flaw::ObserveAfterALostAnswerIsAbsent), Ok(_)) if self.lost => {
                Ok(Presence::Absent)
            }
            (Some(Flaw::BurnedReadsAsAbsent), Ok(Presence::Burned)) => Ok(Presence::Absent),
            (Some(Flaw::EchoesTheDeclaredDigest), Ok(found)) => {
                let declared = self.declared.iter().rev().find(|(c, _)| c == k);
                let echo = |held: Held| match declared {
                    Some((_, digest)) => digest_of(held, digest),
                    None => held,
                };
                Ok(match found {
                    Presence::Present(held) => Presence::Present(echo(held)),
                    Presence::Staged(held) => Presence::Staged(echo(held)),
                    other => other,
                })
            }
            (_, result) => result,
        }
    }

    fn stage(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<StageAnswer, SeamError> {
        self.calls += 1;
        let coordinates: Vec<Coordinate> = items.iter().map(|i| i.coordinate.clone()).collect();
        if self.holds_none_of(&coordinates) {
            return Err(SeamError::Refused(Reason::Channel));
        }
        if self.sealed(&coordinates) {
            return self.answer_to_a_sealed_version(StageAnswer::Duplicate);
        }
        let staged_before = items
            .first()
            .and_then(|item| self.staged_bytes(item.coordinate));
        self.remember(items);
        if let Some(first) = items.first() {
            let held = self.inner.bytes(first.coordinate).map(<[u8]>::to_vec);
            let ours = first.file.bytes.to_vec();
            let yanked = self.inner.is_yanked(first.coordinate);
            if self.is(Flaw::StageOverOwnStagedSameBytesIsStaged)
                && staged_before.as_ref() == Some(&ours)
            {
                return Ok(StageAnswer::Staged);
            }
            if self.is(Flaw::StageOverOwnStagedOverwrites)
                && staged_before.as_ref().is_some_and(|theirs| *theirs != ours)
            {
                let grant = self.internal(GrantPhase::Stage, &coordinates);
                self.inner.abort(&grant, &coordinates)?;
                return self.inner.stage(g, items);
            }
            if self.is(Flaw::StageOverTheSameBytesIsStaged)
                && held.as_ref() == Some(&ours)
                && !yanked
            {
                return Ok(StageAnswer::Staged);
            }
        }
        self.inner.stage(g, items)
    }

    fn abort(&mut self, g: &Grant, ks: &[Coordinate]) -> Result<(), SeamError> {
        self.calls += 1;
        if self.holds_none_of(ks) {
            return Err(SeamError::Refused(Reason::Channel));
        }
        // A stub that breaks what an abort does still refuses a grant it should refuse.
        if self.is(Flaw::AbortDoesNothing) && self.is_good_for(g, "stage", ks) {
            return Ok(());
        }
        if self.is(Flaw::AbortGivesBackAllStaged) && scope_of(g) == "stage" {
            self.inner.abort(g, ks)?;
            let all = self.known_where(|me, c| me.inner.is_staged(c));
            let grant = self.internal(GrantPhase::Stage, &all);
            return self.inner.abort(&grant, &all);
        }
        self.inner.abort(g, ks)
    }

    fn commit(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<CommitAnswer, SeamError> {
        self.calls += 1;
        let coordinates: Vec<Coordinate> = items.iter().map(|i| i.coordinate.clone()).collect();
        if self.holds_none_of(&coordinates) {
            return if self.is(Flaw::UnrepresentableCommitIsPermanent) {
                Err(SeamError::Permanent(
                    "the coordinate is not held".to_owned(),
                ))
            } else {
                Err(SeamError::Refused(Reason::Channel))
            };
        }
        if self.sealed(&coordinates) {
            return self.answer_to_a_sealed_version(CommitAnswer::Duplicate);
        }
        self.remember(items);
        self.commit_calls += 1;
        let first = items.first();
        let was_staged = first.is_some_and(|item| self.inner.is_staged(item.coordinate));
        if let Some(item) = first {
            let held = self.inner.bytes(item.coordinate).map(<[u8]>::to_vec);
            let ours = item.file.bytes.to_vec();
            if self.is(Flaw::CommitOverYankedCommits)
                && self.shape.stages
                && self.inner.is_yanked(item.coordinate)
            {
                return Ok(CommitAnswer::Committed);
            }
            if self.is(Flaw::SecondCommitOfTheSameBytesIsCommitted)
                && held.as_ref() == Some(&ours)
                && !self.inner.is_yanked(item.coordinate)
                && !self.becoming
            {
                return Ok(CommitAnswer::Committed);
            }
            if self.is(Flaw::CommitOverOtherBytesOverwrites)
                && !was_staged
                && held.is_some_and(|theirs| theirs != ours)
            {
                self.inner.present_different(item.coordinate, &ours);
                return Ok(CommitAnswer::Committed);
            }
        }
        let result = self.inner.commit(g, items);
        match (self.flaw, result) {
            (Some(Flaw::VanishedStateIsCommitted), Err(SeamError::Permanent(_)))
                if self.vanished =>
            {
                Ok(CommitAnswer::Committed)
            }
            (Some(Flaw::RetryWhileBecomingReadableIsPermanent), Ok(CommitAnswer::Duplicate))
                if self.becoming =>
            {
                Err(SeamError::Permanent("the commit is a repeat".to_owned()))
            }
            (Some(Flaw::BurnedCommitIsCommitted), Ok(CommitAnswer::Duplicate))
                if first.is_some_and(|item| self.burned.contains(item.coordinate)) =>
            {
                Ok(CommitAnswer::Committed)
            }
            (_, Err(SeamError::Unknown(why))) => {
                self.lost = true;
                Err(SeamError::Unknown(why))
            }
            (_, result) => result,
        }
    }

    fn yank(&mut self, g: &Grant, k: &Coordinate, reason: &str) -> Result<(), SeamError> {
        self.calls += 1;
        if self.holds_none_of(std::slice::from_ref(k)) {
            return Err(SeamError::Refused(Reason::Channel));
        }
        if self.version_yank && self.inner.bytes(k).is_some() {
            let held = self.held_of_the_version(k);
            if self.is(Flaw::VersionYankIsPerCoordinate) {
                return self.inner.yank(g, k, reason);
            }
            if !self.is(Flaw::VersionYankIgnoresTheGrant) && !self.is_good_for(g, "commit", &held) {
                return Err(SeamError::GrantRefused(GrantFault::Denied));
            }
            for sibling in held {
                let grant = self.internal(GrantPhase::Commit, std::slice::from_ref(&sibling));
                self.inner.yank(&grant, &sibling, reason)?;
            }
            return Ok(());
        }
        self.inner.yank(g, k, reason)
    }

    fn fetch(&mut self, g: &Grant, k: &Coordinate) -> Result<Option<Vec<u8>>, SeamError> {
        self.calls += 1;
        if self.holds_none_of(std::slice::from_ref(k)) {
            return Err(SeamError::Refused(Reason::Channel));
        }
        if self.is(Flaw::YankedIsNotFetchable) && self.inner.is_yanked(k) {
            return Ok(None);
        }
        let found = self.inner.fetch(g, k)?;
        if found.is_none()
            && matches!(
                self.flaw,
                Some(
                    Flaw::FetchIgnoresVersion
                        | Flaw::FetchIgnoresPlatform
                        | Flaw::FetchIgnoresChannel
                )
            )
            && let Some(sibling) = self.sibling(k)
        {
            return self.inner.fetch(&Grant::none(), &sibling);
        }
        Ok(found)
    }

    fn newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError> {
        self.inner.newest(g, ch, name)
    }

    fn list(&mut self, g: &Grant, scope: &ListScope) -> Result<Vec<Listed>, SeamError> {
        let mut listed = self.inner.list(g, scope)?;
        if self.is(Flaw::ListIgnoresTheChannel) || self.is(Flaw::ListIgnoresThePrefix) {
            let by_channel = self.is(Flaw::ListIgnoresThePrefix);
            let held = self.known_where(|me, c| {
                me.inner.bytes(c).is_some()
                    && if by_channel {
                        c.channel == scope.channel
                    } else {
                        c.name.starts_with(&scope.prefix)
                    }
            });
            listed = held
                .into_iter()
                .filter_map(|coordinate| {
                    self.held_now(&coordinate)
                        .map(|held| Listed { coordinate, held })
                })
                .collect();
        }
        for entry in &mut listed {
            entry.held = self.misreported(entry.held.clone(), Flaw::ListWrongDigest);
            if self.is(Flaw::ListDropsTheStamp) {
                entry.held.stamp = None;
            }
        }
        Ok(listed)
    }
}
