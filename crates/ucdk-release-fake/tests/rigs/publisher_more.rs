// SPDX-License-Identifier: Apache-2.0
//! More stubs of a registry, each around the conforming fake: it takes a call the fake refuses,
//! or answers one differently, in one way and no other.

use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::{ArtifactType, Coordinate, Stamped};
use ucdk_release_seams::kit::{
    Arrangement, ChannelShape, Payload, PublisherRig, ReadStep, Sibling, federated_repository,
};
use ucdk_release_seams::{
    Backoff, CommitAnswer, CommitUnit, Grant, ListScope, Listed, Publisher, SeamError, StageAnswer,
    StageItem, YankUnit,
};

use super::publisher::{FakePublisherRig, Flawed};
use super::publisher_flaw::Behind;
use super::publisher_grant::issue;
use super::publisher_lax::{Defect, Kind, Lax, Pair, Verb};
use super::secret::texts;

/// One field of a coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Channel,
    Version,
    Platform,
    Type,
}

impl Field {
    pub const ALL: [Self; 4] = [Self::Channel, Self::Version, Self::Platform, Self::Type];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum More {
    /// A call whose grant names a coordinate that differs from the one asked in the field only
    /// is taken for one that names it.
    GrantBlindToField(Field, Verb),
    /// On a channel of the kind, a bad grant of the defect is taken for a good one by the verb.
    BadGrantTakenOnShape(Kind, Defect, Verb),
    /// An abort is taken from a commit grant.
    AbortTakesCommitGrant,
    /// The conforming fake, wrapped and changed in no way.
    Conforming,
    /// On a channel that stages, a commit that lost a race replaces the bytes that won.
    StagedRaceOverwrites,
    /// On a channel that does not stage, a commit that names one coordinate twice commits the last
    /// of them.
    NonStagingDupCommitDeduped,
    /// A commit of staged state that vanished is answered `RateLimited`, which promises no effect
    /// and is retried, and not `Refused` or `Permanent`.
    VanishedCommitIsRateLimited,
    /// A commit repeated while the first is becoming readable is written again with the stamp of
    /// the repeat, and answered `Committed`.
    RetryWhileBecomingReplacesTheStamp,
    /// The same, with the exceptional mark of the repeat.
    RetryWhileBecomingReplacesTheMark,
    /// A fetch by a stranger finds nothing, whatever the channel holds.
    FetchOfAStrangerFindsNothing,
    /// A commit of other bytes or another `Held` than was staged replaces the staged state.
    CommitReplacesStaged,
    /// A commit of a coordinate nobody staged, on a channel that stages, stages it first.
    CommitUnstaged,
    /// A commit of a set that meets a held coordinate commits the free ones and answers
    /// `Duplicate`.
    CommitPartial,
    /// A stage of a set that meets a held coordinate stages the free ones and answers
    /// `Duplicate`.
    StagePartial,
    /// A yank also yanks the coordinates of the name that differ in the field.
    YankSiblings(Field),
    /// An abort also gives back the staged coordinates of the name that differ in the field.
    AbortSiblings(Field),
    /// A fetch with a stage or commit grant answers staged bytes.
    FetchWithWriterGrantSeesStaged,
    /// A newest with a stage or commit grant names staged coordinates.
    NewestWithWriterGrantIncludesStaged,
    /// A list with a stage or commit grant names staged coordinates.
    ListWithWriterGrantIncludesStaged,
    /// On a channel that does not stage, a commit that lost a race replaces the bytes that won.
    NonStagingRaceOverwrites,
    /// On a channel that does not stage, a commit over a yanked coordinate commits.
    NonStagingCommitOverYanked,
    /// On a channel that does not stage, a failure after the commit landed is `Permanent`.
    NonStagingTransientIsPermanent,
    /// On a channel that does not stage, a set commit that fails leaves the first coordinate
    /// readable.
    NonStagingSetCommitIsTorn,
    /// A yank of a coordinate nobody holds is answered `Ok`.
    YankAbsentOk,
    /// A stage of a coordinate that was aborted is answered `Duplicate`.
    StageAfterAbortIsDuplicate,
    /// A stage that names one coordinate twice stages the last of them.
    DupStageDeduped,
    /// A commit that names one coordinate twice commits the last of them.
    DupCommitDeduped,
    /// An observe told a set that names one coordinate twice answers.
    DupObserveSetAccepted,
    /// A stage is `Duplicate` when a coordinate of the name that differs in the field is held or
    /// staged.
    StageSiblingTaken(Field),
    /// A commit is `Duplicate` when a coordinate of the name that differs in the field is held.
    CommitSiblingTaken(Field),
    /// An observe of a coordinate nobody staged is `Staged` when one of the name that differs in
    /// the field is staged.
    ObserveStagedSibling(Field),
    /// A commit of a set that is refused for one coordinate still applies the others.
    CommitPartialOnRefusal,
    /// A stage of a set that is refused for one coordinate still stages the others.
    StagePartialOnRefusal,
    /// A yank also yanks the other coordinates of the commit that made the one it names.
    YankWidensToTheCommitSet,
    /// A yank also yanks every held coordinate whose name starts with the name it names.
    YankByNamePrefix,
    /// An observe of a coordinate nobody holds answers a held coordinate whose name extends its
    /// name.
    ObserveByNamePrefix,
    /// A fetch of a coordinate nobody holds answers the bytes of a held coordinate whose name
    /// extends its name.
    FetchByNamePrefix,
    /// A commit of one coordinate publishes the bytes another staged coordinate holds.
    CommitPublishesOtherStagedBytes,
    /// A commit that is answered `Duplicate` replaces the stamp the coordinate holds.
    DuplicateRestamps,
    /// An abort also drops the committed coordinates it does not name.
    AbortDropsUnnamedCommitted,
    /// While the listing lags, an observe of a coordinate another writer committed reads it.
    ObserveOfOthersReadsTheLaggingList,
    /// An observe echoes the digest the writer declared for a coordinate of a set of several.
    EchoesOnAMultiMemberSet,
    /// An observe of a yanked coordinate echoes the digest the writer declared.
    EchoesOnAYankedCoordinate,
    /// A stage of a set that is answered `Duplicate` replaces the bytes of the coordinate another
    /// writer holds with its own.
    StageDupOverwritesHeld,
    /// A stage of a set that is answered `Duplicate` yanks the coordinate another writer holds.
    StageDupYanksHeld,
    /// A commit of a set that is answered `Duplicate` yanks the coordinate another writer holds.
    CommitDupYanksHeld,
    /// A commit of one coordinate publishes the bytes staged at a coordinate of its name that
    /// differs in a field.
    CommitPublishesSiblingStagedBytes,
    /// An abort also yanks the committed coordinates of the name it names that differ in a field.
    AbortWithdrawsCommittedSibling,
    /// A stage, a commit or an abort of three coordinates checks the grant for two of them only.
    CoverageOfTwo(Verb, Pair),
    /// On a channel of the kind, a read grant is taken for the grant of the write.
    ReadGrantWrites(Kind, Verb),
    /// On a channel of the kind, a read that is a `fetch`, a `newest` or a `list` and carries a
    /// stage or a commit grant answers staged state.
    WriterSeesStaged(Kind, Verb),
    /// On a channel of the kind, an observe with a read grant sees the staged state of the
    /// caller, told the set when the channel is addressed by group.
    ReadSeesStaged(Kind),
    /// An observe with a read grant sees the staged state of another writer.
    ReadSeesOthersStaged,
    /// A stub that is lax in one way on one kind of channel.
    Lax(Lax),
}

/// The siblings the rig names, whatever the rig it wraps names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Siblings {
    /// Those of the rig it wraps.
    Base,
    /// None.
    Nobody,
    /// Only the later platform of the base, which a registry that seals a version cannot publish
    /// beside it.
    OnlyAPlatform,
}

pub struct MoreRig {
    base: FakePublisherRig,
    flaw: More,
    /// The stub is behind the channels of this kind only, when the rig says.
    behind: Option<Behind>,
    siblings: Siblings,
}

impl MoreRig {
    /// The same rig naming no sibling of a coordinate.
    #[must_use]
    pub fn naming_no_sibling(self) -> Self {
        Self {
            siblings: Siblings::Nobody,
            ..self
        }
    }

    /// The same rig naming one sibling, a later platform, in a set of its own.
    #[must_use]
    pub fn naming_only_a_platform(self) -> Self {
        Self {
            siblings: Siblings::OnlyAPlatform,
            ..self
        }
    }

    pub fn flawed(flaw: More) -> Self {
        Self::flawed_on(FakePublisherRig::conforming(), flaw)
    }

    /// The stub of `flaw` around the rig `base`.
    pub fn flawed_on(base: FakePublisherRig, flaw: More) -> Self {
        Self {
            base,
            flaw,
            behind: None,
            siblings: Siblings::Base,
        }
    }

    /// The stub of `flaw` around the rig `base`, behind the channels of one kind only: the
    /// channels of the other shapes are the conforming fake.
    pub fn flawed_behind(base: FakePublisherRig, flaw: More, behind: Behind) -> Self {
        Self {
            behind: Some(behind),
            ..Self::flawed_on(base, flaw)
        }
    }
}

pub struct Wrapped {
    pub(super) base: Flawed,
    pub(super) flaw: More,
    /// The stub is in force on this channel.
    active: bool,
    pub(super) shape: ChannelShape,
    /// The digest each coordinate was declared with, and how many coordinates the set had.
    declared: Vec<(Coordinate, String, usize)>,
    /// The coordinates of each commit that landed.
    sets: Vec<Vec<Coordinate>>,
    /// Committed coordinates an abort dropped.
    dropped: Vec<Coordinate>,
    /// The stamp the last commit answered `Duplicate` declared for a coordinate.
    restamped: Vec<(Coordinate, Option<Stamped>)>,
    /// Coordinates another writer committed.
    pub(super) others: Vec<Coordinate>,
    /// The listing lags.
    lagging: bool,
    /// A commit was raced.
    raced: bool,
    /// The staged state was arranged to vanish.
    vanished: bool,
    /// A commit was arranged to become readable late.
    becoming: bool,
    /// The stamp and the mark of the last repeat of a commit that was written again.
    rewritten: Vec<(Coordinate, Option<Stamped>, bool)>,
    /// The coordinates of every call that went through, before the call that is running.
    known: Vec<Coordinate>,
    pub(super) committed: Vec<Coordinate>,
    pub(super) staged: Vec<(Coordinate, Vec<u8>)>,
    aborted: Vec<Coordinate>,
    /// Coordinates another writer staged, with the bytes it staged.
    pub(super) staged_by_others: Vec<(Coordinate, Vec<u8>)>,
}

impl PublisherRig for MoreRig {
    type Publisher = Wrapped;

    fn shapes(&self) -> Vec<ChannelShape> {
        self.base.shapes()
    }

    fn publisher(&self, shape: &ChannelShape) -> Wrapped {
        Wrapped {
            base: self.base.publisher(shape),
            flaw: self.flaw,
            active: self.behind.is_none_or(|behind| behind.holds(shape)),
            shape: *shape,
            declared: Vec::new(),
            sets: Vec::new(),
            dropped: Vec::new(),
            restamped: Vec::new(),
            others: Vec::new(),
            lagging: false,
            raced: false,
            vanished: false,
            becoming: false,
            rewritten: Vec::new(),
            known: Vec::new(),
            committed: Vec::new(),
            staged: Vec::new(),
            aborted: Vec::new(),
            staged_by_others: Vec::new(),
        }
    }

    fn arrange(&self, p: &mut Wrapped, what: &Arrangement) {
        if matches!(what, Arrangement::RaceNextCommit(_)) {
            p.raced = true;
        }
        match what {
            Arrangement::ListLags => p.lagging = true,
            Arrangement::LoseStagedState => p.vanished = true,
            Arrangement::CommitVisibleAfter(_) => p.becoming = true,
            Arrangement::HeldByAnother { coordinate, .. } => p.others.push(coordinate.clone()),
            Arrangement::StagedByAnother { coordinate, bytes } => {
                p.staged_by_others.push((coordinate.clone(), bytes.clone()));
            }
            _ => {}
        }
        if self.flaw == More::NonStagingSetCommitIsTorn
            && p.active
            && !p.shape.stages
            && matches!(what, Arrangement::CommitSetFailsWhole)
        {
            p.base.inner.torn_commit();
            return;
        }
        self.base.arrange(&mut p.base, what);
    }

    fn grant(&self, p: &Wrapped, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        self.base.grant(&p.base, phase, coordinates)
    }

    fn foreign_grant(&self, p: &Wrapped, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        self.base.foreign_grant(&p.base, phase, coordinates)
    }

    fn advance(&self, ms: u64) {
        self.base.advance(ms);
    }

    fn reads(&self, p: &Wrapped) -> Vec<ReadStep> {
        self.base.reads(&p.base)
    }

    fn sent(&self, p: &Wrapped) -> usize {
        self.base.sent(&p.base)
    }

    fn coordinate(&self, label: &str) -> Coordinate {
        self.base.coordinate(label)
    }

    fn payload(&self, k: &Coordinate, seed: u8) -> Payload {
        self.base.payload(k, seed)
    }

    fn siblings(&self, base: &Coordinate) -> Vec<Sibling> {
        match self.siblings {
            Siblings::Base => self.base.siblings(base),
            Siblings::Nobody => Vec::new(),
            Siblings::OnlyAPlatform => self
                .base
                .siblings(base)
                .into_iter()
                .filter(|s| s.field == "platform")
                .collect(),
        }
    }

    fn unrepresentable(&self) -> Vec<Coordinate> {
        self.base.unrepresentable()
    }

    fn seals_versions(&self) -> bool {
        self.base.seals_versions()
    }

    fn can_burn(&self) -> bool {
        self.base.can_burn()
    }
}

/// Whether `a` and `b` are two coordinates of one name that differ in `field` and in no other.
pub(super) fn differs_only_in(a: &Coordinate, b: &Coordinate, field: Field) -> bool {
    let agree = |same: bool, of: Field| if field == of { !same } else { same };
    a.name == b.name
        && agree(a.channel == b.channel, Field::Channel)
        && agree(a.version == b.version, Field::Version)
        && agree(a.platform == b.platform, Field::Platform)
        && agree(a.r#type == b.r#type, Field::Type)
}

/// `items` without a coordinate that a later item names again.
fn deduped<'a>(items: &[StageItem<'a>]) -> Vec<StageItem<'a>> {
    items
        .iter()
        .enumerate()
        .filter(|(at, item)| {
            !items[at + 1..]
                .iter()
                .any(|later| later.coordinate == item.coordinate)
        })
        .map(|(_, item)| *item)
        .collect()
}

/// Whether `g` is a stage or a commit grant.
pub(super) fn is_writer(g: &Grant) -> bool {
    matches!(
        texts(g.secret()).first().map(String::as_str),
        Some("stage" | "commit")
    )
}

impl Wrapped {
    fn is(&self, flaw: More) -> bool {
        self.active && self.flaw == flaw
    }

    /// The flaw of the stub, when it is in force on this channel.
    pub(super) fn flaw(&self) -> Option<More> {
        self.active.then_some(self.flaw)
    }

    /// A committed coordinate that differs from `k` in its name alone, which extends the name of
    /// `k`.
    fn extending(&self, k: &Coordinate) -> Option<Coordinate> {
        self.committed
            .iter()
            .find(|c| {
                c.name != k.name
                    && c.name.starts_with(&k.name)
                    && c.channel == k.channel
                    && c.version == k.version
                    && c.platform == k.platform
                    && c.r#type == k.r#type
            })
            .cloned()
    }

    /// The digest the writer declared for `k`, when there was one.
    /// Whether `result` is a refusal, which promises that nothing happened, of a set that does
    /// not name a coordinate twice: a registry that checks that first and then applies the
    /// members one at a time.
    fn refused_for_a_member(result: &Result<impl Sized, SeamError>) -> bool {
        result.as_ref().is_err_and(|e| {
            e.promises_no_effect()
                && !matches!(e, SeamError::Permanent(why) if why.contains("twice"))
        })
    }

    fn declared_for(&self, k: &Coordinate) -> Option<&(Coordinate, String, usize)> {
        self.declared.iter().rev().find(|(c, _, _)| c == k)
    }

    fn remember_declared(&mut self, items: &[StageItem<'_>]) {
        for item in items {
            if !item.held.digest.is_empty() {
                self.declared.push((
                    item.coordinate.clone(),
                    item.held.digest.clone(),
                    items.len(),
                ));
            }
        }
    }

    pub(super) fn internal(&self, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        issue(
            &self.base.clock,
            &federated_repository(),
            phase,
            coordinates,
        )
    }

    /// The coordinates of `items` that another writer holds.
    fn held_of_others(&self, items: &[StageItem<'_>]) -> Vec<Coordinate> {
        items
            .iter()
            .filter(|i| self.others.contains(i.coordinate))
            .map(|i| i.coordinate.clone())
            .collect()
    }

    /// Yanks each of `coordinates`, which the registry has no business to touch.
    fn yank_all(&mut self, coordinates: &[Coordinate]) {
        for k in coordinates {
            let grant = self.internal(GrantPhase::Commit, std::slice::from_ref(k));
            let _ = self.base.yank(&grant, k, "a wrong implementation");
        }
    }

    /// Whether a coordinate a call went through with is held or staged and is of the name of
    /// `c`, differing in `field`.
    fn sibling_in_place(&self, c: &Coordinate, field: Field, held_only: bool) -> bool {
        self.known.iter().any(|k| {
            differs_only_in(k, c, field)
                && (self.base.inner.bytes(k).is_some()
                    || (!held_only && self.base.inner.is_staged(k)))
        })
    }
}

impl Publisher for Wrapped {
    fn types(&self) -> &[ArtifactType] {
        self.base.types()
    }

    fn stages(&self) -> bool {
        self.base.stages()
    }

    fn commit_unit(&self) -> CommitUnit {
        self.base.commit_unit()
    }

    fn conditional_put(&self) -> bool {
        self.base.conditional_put()
    }

    fn grant_limit(&self) -> usize {
        self.base.grant_limit()
    }

    fn yank_unit(&self) -> YankUnit {
        self.base.yank_unit()
    }

    fn observe(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        set: &[StageItem<'_>],
    ) -> Result<Presence, SeamError> {
        if self.is(More::AbortDropsUnnamedCommitted) && self.dropped.contains(k) {
            return Ok(Presence::Absent);
        }
        if self.is(More::ObserveOfOthersReadsTheLaggingList)
            && self.lagging
            && self.others.contains(k)
        {
            return Ok(Presence::Absent);
        }
        let mut result = self.base.observe(g, k, set);
        let retry = self.lenient(Verb::Observe, g, std::slice::from_ref(k), &result);
        if let Some(grant) = retry {
            result = self.base.observe(&grant, k, set);
        }
        if self.is(More::DupObserveSetAccepted) && result.is_err() {
            let once = deduped(set);
            if once.len() != set.len() {
                result = self.base.observe(g, k, &once);
            }
        }
        if self.is(More::ObserveByNamePrefix)
            && matches!(result, Ok(Presence::Absent))
            && let Some(longer) = self.extending(k)
        {
            result = self.base.observe(&Grant::none(), &longer, &[]);
        }
        if self.is(More::DuplicateRestamps)
            && let Ok(Presence::Present(held)) = &mut result
            && let Some((_, stamp)) = self.restamped.iter().rev().find(|(c, _)| c == k)
        {
            held.stamp.clone_from(stamp);
        }
        if let Ok(Presence::Present(held)) = &mut result
            && let Some((_, stamp, mark)) = self.rewritten.iter().rev().find(|(c, _, _)| c == k)
        {
            if self.is(More::RetryWhileBecomingReplacesTheStamp) {
                held.stamp.clone_from(stamp);
            }
            if self.is(More::RetryWhileBecomingReplacesTheMark) {
                held.exceptional = *mark;
            }
        }
        let echoes = match (&result, self.declared_for(k)) {
            (Ok(Presence::Present(_) | Presence::Staged(_)), Some((_, digest, members))) => {
                (self.is(More::EchoesOnAMultiMemberSet) && *members > 1).then(|| digest.clone())
            }
            (Ok(Presence::Yanked(_)), Some((_, digest, _))) => self
                .is(More::EchoesOnAYankedCoordinate)
                .then(|| digest.clone()),
            _ => None,
        };
        if let Some(digest) = echoes
            && let Ok(Presence::Present(held) | Presence::Staged(held) | Presence::Yanked(held)) =
                &mut result
        {
            held.digest = digest;
        }
        if let Some(More::ObserveStagedSibling(field)) = self.flaw()
            && matches!(result, Ok(Presence::Absent))
            && !g.is_none()
            && self
                .staged
                .iter()
                .any(|(c, _)| differs_only_in(c, k, field) && self.base.inner.is_staged(c))
        {
            result = Ok(Presence::Staged(Held::default()));
        }
        if matches!(result, Ok(Presence::Absent))
            && self.read_grant_sees_staged(g, k, !set.is_empty())
        {
            result = Ok(Presence::Staged(Held::default()));
        }
        self.lax_observe(g, k, set, result)
    }

    fn stage(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<StageAnswer, SeamError> {
        let coordinates: Vec<Coordinate> = items.iter().map(|i| i.coordinate.clone()).collect();
        let used = if self.is(More::DupStageDeduped) {
            deduped(items)
        } else {
            items.to_vec()
        };
        self.remember_declared(items);
        let mut result = self.base.stage(g, &used);
        let retry = self.lenient(Verb::Stage, g, &coordinates, &result);
        if let Some(grant) = retry {
            result = self.base.stage(&grant, &used);
        }
        if self.is(More::StagePartialOnRefusal)
            && items.len() > 1
            && Self::refused_for_a_member(&result)
        {
            for item in items {
                let grant = self.internal(GrantPhase::Stage, std::slice::from_ref(item.coordinate));
                let _ = self.base.stage(&grant, std::slice::from_ref(item));
            }
        }
        if result == Ok(StageAnswer::Duplicate) && self.is(More::StagePartial) {
            for item in items {
                let one = [item.coordinate.clone()];
                let grant = self.internal(GrantPhase::Stage, &one);
                let _ = self.base.stage(&grant, std::slice::from_ref(item));
            }
        }
        if result == Ok(StageAnswer::Duplicate) && items.len() > 1 {
            let held = self.held_of_others(items);
            if self.is(More::StageDupOverwritesHeld) {
                for item in items.iter().filter(|i| held.contains(i.coordinate)) {
                    self.base
                        .inner
                        .present_different(item.coordinate, item.file.bytes);
                }
            }
            if self.is(More::StageDupYanksHeld) {
                self.yank_all(&held);
            }
        }
        if result == Ok(StageAnswer::Staged) {
            let taken = match self.flaw() {
                Some(More::StageAfterAbortIsDuplicate) => {
                    coordinates.iter().any(|c| self.aborted.contains(c))
                }
                Some(More::StageSiblingTaken(field)) => coordinates
                    .iter()
                    .any(|c| self.sibling_in_place(c, field, false)),
                _ => false,
            };
            for item in &used {
                self.staged
                    .push((item.coordinate.clone(), item.file.bytes.to_vec()));
            }
            self.known.extend(coordinates);
            if taken {
                result = Ok(StageAnswer::Duplicate);
            }
        }
        result
    }

    fn abort(&mut self, g: &Grant, ks: &[Coordinate]) -> Result<(), SeamError> {
        let mut result = self.base.abort(g, ks);
        let mut retry = self.lenient(Verb::Abort, g, ks, &result);
        if retry.is_none()
            && self.is(More::AbortTakesCommitGrant)
            && result.is_err()
            && texts(g.secret()).first().map(String::as_str) == Some("commit")
        {
            retry = Some(self.internal(GrantPhase::Stage, ks));
        }
        if let Some(grant) = retry {
            result = self.base.abort(&grant, ks);
        }
        if result.is_ok() {
            self.aborted.extend(ks.iter().cloned());
            if self.is(More::AbortDropsUnnamedCommitted) {
                let unnamed: Vec<Coordinate> = self
                    .committed
                    .iter()
                    .filter(|c| !ks.contains(c))
                    .cloned()
                    .collect();
                self.dropped.extend(unnamed);
            }
            if self.is(More::AbortWithdrawsCommittedSibling) {
                let siblings: Vec<Coordinate> = self
                    .committed
                    .iter()
                    .filter(|c| !ks.contains(c) && ks.iter().any(|k| c.name == k.name))
                    .cloned()
                    .collect();
                self.yank_all(&siblings);
            }
            if let Some(More::AbortSiblings(field)) = self.flaw() {
                let siblings: Vec<Coordinate> = self
                    .staged
                    .iter()
                    .map(|(c, _)| c.clone())
                    .filter(|c| {
                        ks.iter().any(|k| differs_only_in(c, k, field))
                            && self.base.inner.is_staged(c)
                    })
                    .collect();
                if !siblings.is_empty() {
                    let grant = self.internal(GrantPhase::Stage, &siblings);
                    let _ = self.base.abort(&grant, &siblings);
                }
            }
        }
        result
    }

    fn commit(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<CommitAnswer, SeamError> {
        let coordinates: Vec<Coordinate> = items.iter().map(|i| i.coordinate.clone()).collect();
        let used = if self.is(More::DupCommitDeduped)
            || (self.is(More::NonStagingDupCommitDeduped) && !self.shape.stages)
        {
            deduped(items)
        } else {
            items.to_vec()
        };
        let yanked_first = items
            .first()
            .is_some_and(|item| self.base.inner.is_yanked(item.coordinate));
        self.remember_declared(items);
        let mut result = self.base.commit(g, &used);
        let retry = self.lenient(Verb::Commit, g, &coordinates, &result);
        if let Some(grant) = retry {
            result = self.base.commit(&grant, &used);
        }
        if self.is(More::CommitPartialOnRefusal)
            && items.len() > 1
            && Self::refused_for_a_member(&result)
        {
            for item in items {
                let grant =
                    self.internal(GrantPhase::Commit, std::slice::from_ref(item.coordinate));
                let _ = self.base.commit(&grant, std::slice::from_ref(item));
            }
        }
        let redo = match &result {
            Err(SeamError::Permanent(why)) => (self.is(More::CommitReplacesStaged)
                && why.contains("differs from what was staged"))
            .then_some(true)
            .or_else(|| {
                (self.is(More::CommitUnstaged) && why.contains("is not staged")).then_some(false)
            }),
            _ => None,
        };
        if let Some(replace) = redo {
            let grant = self.internal(GrantPhase::Stage, &coordinates);
            if replace {
                let _ = self.base.abort(&grant, &coordinates);
            }
            let _ = self.base.stage(&grant, items);
            result = self.base.commit(g, items);
        }
        if result == Ok(CommitAnswer::Duplicate) {
            if items.len() > 1 && self.is(More::CommitDupYanksHeld) {
                let held = self.held_of_others(items);
                self.yank_all(&held);
            }
            if self.is(More::DuplicateRestamps) {
                for item in items {
                    self.restamped
                        .push((item.coordinate.clone(), item.held.stamp.clone()));
                }
            }
            if self.is(More::CommitPartial) && items.len() > 1 {
                for item in items {
                    let one = [item.coordinate.clone()];
                    let grant = self.internal(GrantPhase::Commit, &one);
                    let _ = self.base.commit(&grant, std::slice::from_ref(item));
                }
            }
            if !self.shape.stages && self.is(More::NonStagingRaceOverwrites) && self.raced {
                for item in items {
                    self.base
                        .inner
                        .present_different(item.coordinate, item.file.bytes);
                }
                result = Ok(CommitAnswer::Committed);
            }
            if !self.shape.stages && self.is(More::NonStagingCommitOverYanked) && yanked_first {
                result = Ok(CommitAnswer::Committed);
            }
            if self.shape.stages && self.is(More::StagedRaceOverwrites) && self.raced {
                for item in items {
                    self.base
                        .inner
                        .present_different(item.coordinate, item.file.bytes);
                }
                result = Ok(CommitAnswer::Committed);
            }
            let rewrites = self.is(More::RetryWhileBecomingReplacesTheStamp)
                || self.is(More::RetryWhileBecomingReplacesTheMark);
            if self.becoming && rewrites {
                for item in items {
                    self.rewritten.push((
                        item.coordinate.clone(),
                        item.held.stamp.clone(),
                        item.held.exceptional,
                    ));
                }
                result = Ok(CommitAnswer::Committed);
            }
        }
        if self.is(More::VanishedCommitIsRateLimited)
            && self.vanished
            && matches!(&result, Err(SeamError::Permanent(why)) if why.contains("is not staged"))
        {
            result = Err(SeamError::RateLimited(Backoff { ticks: 1 }));
        }
        if !self.shape.stages
            && self.is(More::NonStagingTransientIsPermanent)
            && let Err(SeamError::Transient(_)) = result
        {
            result = Err(SeamError::Permanent("the commit failed".to_owned()));
        }
        if result == Ok(CommitAnswer::Committed) {
            let taken = match self.flaw() {
                Some(More::CommitSiblingTaken(field)) => coordinates.iter().any(|c| {
                    self.committed
                        .iter()
                        .any(|held| differs_only_in(held, c, field))
                }),
                _ => false,
            };
            if self.is(More::CommitPublishesOtherStagedBytes)
                && let [item] = items
                && let Some((_, theirs)) = self
                    .staged
                    .iter()
                    .find(|(c, _)| c != item.coordinate && self.base.inner.is_staged(c))
            {
                self.base.inner.present_different(item.coordinate, theirs);
            }
            if self.is(More::CommitPublishesSiblingStagedBytes)
                && let [item] = items
                && let Some((_, theirs)) = self.staged.iter().find(|(c, _)| {
                    c != item.coordinate
                        && c.name == item.coordinate.name
                        && self.base.inner.is_staged(c)
                })
            {
                self.base.inner.present_different(item.coordinate, theirs);
            }
            self.sets.push(coordinates.clone());
            self.committed.extend(coordinates.iter().cloned());
            self.staged.retain(|(c, _)| !coordinates.contains(c));
            self.known.extend(coordinates);
            if taken {
                result = Ok(CommitAnswer::Duplicate);
            }
        }
        result
    }

    fn yank(&mut self, g: &Grant, k: &Coordinate, reason: &str) -> Result<(), SeamError> {
        let mut result = self.base.yank(g, k, reason);
        let retry = self.lenient(Verb::Yank, g, std::slice::from_ref(k), &result);
        if let Some(grant) = retry {
            result = self.base.yank(&grant, k, reason);
        }
        if self.is(More::YankAbsentOk)
            && let Err(SeamError::Permanent(why)) = &result
            && why.contains("is not held")
        {
            result = Ok(());
        }
        if result.is_ok() {
            let widened: Vec<Coordinate> = if self.is(More::YankWidensToTheCommitSet) {
                self.sets
                    .iter()
                    .filter(|set| set.contains(k))
                    .flatten()
                    .filter(|c| *c != k)
                    .cloned()
                    .collect()
            } else if self.is(More::YankByNamePrefix) {
                self.committed
                    .iter()
                    .filter(|c| c.name != k.name && c.name.starts_with(&k.name))
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            };
            for other in widened {
                let grant = self.internal(GrantPhase::Commit, std::slice::from_ref(&other));
                let _ = self.base.yank(&grant, &other, reason);
            }
        }
        if let (Ok(()), Some(More::YankSiblings(field))) = (&result, self.flaw()) {
            let siblings: Vec<Coordinate> = self
                .committed
                .iter()
                .filter(|c| differs_only_in(c, k, field))
                .cloned()
                .collect();
            for sibling in siblings {
                let grant = self.internal(GrantPhase::Commit, std::slice::from_ref(&sibling));
                let _ = self.base.yank(&grant, &sibling, reason);
            }
        }
        result
    }

    fn fetch(&mut self, g: &Grant, k: &Coordinate) -> Result<Option<Vec<u8>>, SeamError> {
        if self.is(More::AbortDropsUnnamedCommitted) && self.dropped.contains(k) {
            return Ok(None);
        }
        if self.is(More::FetchOfAStrangerFindsNothing) && g.is_none() {
            return Ok(None);
        }
        let mut result = self.base.fetch(g, k);
        let retry = self.lenient(Verb::Fetch, g, std::slice::from_ref(k), &result);
        if let Some(grant) = retry {
            result = self.base.fetch(&grant, k);
        }
        if self.is(More::FetchByNamePrefix)
            && matches!(result, Ok(None))
            && let Some(longer) = self.extending(k)
        {
            result = self.base.fetch(&Grant::none(), &longer);
        }
        if (self.is(More::FetchWithWriterGrantSeesStaged) || self.writer_sees_staged(Verb::Fetch))
            && matches!(result, Ok(None))
            && is_writer(g)
            && self.base.inner.is_staged(k)
            && let Some((_, bytes)) = self.staged.iter().rev().find(|(c, _)| c == k)
        {
            result = Ok(Some(bytes.clone()));
        }
        self.lax_fetch(g, k, result)
    }

    fn newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError> {
        let mut result = self.base.newest(g, ch, name);
        let retry = self.lenient(Verb::Newest, g, &[], &result);
        if let Some(grant) = retry {
            result = self.base.newest(&grant, ch, name);
        }
        let mut found = result?;
        if (self.is(More::NewestWithWriterGrantIncludesStaged)
            || self.writer_sees_staged(Verb::Newest))
            && is_writer(g)
        {
            for (c, _) in self.staged.clone() {
                if c.channel == ch && c.name == name && self.base.inner.is_staged(&c) {
                    // A channel addressed by group shows no staged state to an observe that is
                    // told no set: what the stub makes up has no digest.
                    let held = self.base.held_now(&c).unwrap_or_default();
                    found.push((c, held));
                }
            }
        }
        Ok(self.lax_newest(g, ch, name, found))
    }

    fn list(&mut self, g: &Grant, scope: &ListScope) -> Result<Vec<Listed>, SeamError> {
        let mut result = self.base.list(g, scope);
        let retry = self.lenient(Verb::List, g, &[], &result);
        if let Some(grant) = retry {
            result = self.base.list(&grant, scope);
        }
        let mut listed = result?;
        if (self.is(More::ListWithWriterGrantIncludesStaged) || self.writer_sees_staged(Verb::List))
            && is_writer(g)
        {
            for (c, _) in self.staged.clone() {
                if c.channel == scope.channel
                    && c.name.starts_with(&scope.prefix)
                    && self.base.inner.is_staged(&c)
                {
                    let held = self.base.held_now(&c).unwrap_or_default();
                    listed.push(Listed {
                        coordinate: c,
                        held,
                    });
                }
            }
        }
        Ok(self.lax_list(g, scope, listed))
    }
}
