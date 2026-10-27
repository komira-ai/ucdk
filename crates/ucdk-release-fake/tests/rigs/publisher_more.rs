// SPDX-License-Identifier: Apache-2.0
//! More stubs of a registry, each around the conforming fake: it takes a call the fake refuses,
//! or answers one differently, in one way and no other.

use ucdk_core::Clock;
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::{Held, Presence};
use ucdk_release_fake::coordinate_key;
use ucdk_release_proto::v1::{ArtifactType, Coordinate};
use ucdk_release_seams::kit::{
    Arrangement, ChannelShape, LOCATION, PublisherRig, ReadStep, federated_repository,
};
use ucdk_release_seams::{
    CommitAnswer, CommitUnit, Grant, ListScope, Listed, Publisher, SeamError, StageAnswer,
    StageItem,
};

use super::publisher::{FakePublisherRig, Flawed, issue};

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
    /// The fields a scenario can vary without changing what kind of artifact the name is.
    pub const SIBLING: [Self; 3] = [Self::Channel, Self::Version, Self::Platform];
}

/// A call of a registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Stage,
    Commit,
    Abort,
    Yank,
    Observe,
    Fetch,
}

impl Verb {
    pub const ALL: [Self; 6] = [
        Self::Stage,
        Self::Commit,
        Self::Abort,
        Self::Yank,
        Self::Observe,
        Self::Fetch,
    ];
}

/// How a grant is bad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defect {
    /// It is for another repository.
    Foreign,
    /// It has expired.
    Expired,
    /// It is of the right phase and does not name the coordinates of the call.
    Uncovered,
    /// It is the grant of the other write phase (a read grant is no write grant of any phase).
    OtherWritePhase,
}

/// The shape of channel a stub is lax on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A private channel that stages and addresses staged state by coordinate.
    Private,
    /// A channel that addresses staged state by group.
    Grouped,
    /// A channel that does not stage.
    NonStaging,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum More {
    /// A call whose grant names a coordinate that differs from the one asked in the field only
    /// is taken for one that names it.
    GrantBlindToField(Field, Verb),
    /// On a channel of the kind, a bad grant of the defect is taken for a good one by the verb.
    BadGrantTakenOnShape(Kind, Defect, Verb),
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
    /// An abort is taken from a commit grant.
    AbortTakesCommitGrant,
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
}

/// Every stub that takes a grant naming a coordinate that differs in one field.
pub fn field_blind() -> Vec<More> {
    let mut all = Vec::new();
    for field in Field::ALL {
        for verb in Verb::ALL {
            all.push(More::GrantBlindToField(field, verb));
        }
    }
    all
}

/// Every stub that is lax about a bad grant on a shape of channel the kit once held it on one
/// shape only.
pub fn on_shapes() -> Vec<More> {
    let mut all = Vec::new();
    for kind in [Kind::Private, Kind::Grouped, Kind::NonStaging] {
        for defect in [
            Defect::Foreign,
            Defect::Expired,
            Defect::Uncovered,
            Defect::OtherWritePhase,
        ] {
            for verb in Verb::ALL {
                let writes = matches!(verb, Verb::Stage | Verb::Commit | Verb::Abort | Verb::Yank);
                // A read has no phase to get wrong; a channel that does not stage is never
                // asked to stage or abort; the reads of a private plain channel are held by the
                // stubs that take a read grant for another.
                let kept = match kind {
                    Kind::Private => writes,
                    Kind::Grouped => writes || defect != Defect::OtherWritePhase,
                    Kind::NonStaging => {
                        matches!(verb, Verb::Commit | Verb::Yank)
                            || (!writes && defect != Defect::OtherWritePhase)
                    }
                };
                if kept {
                    all.push(More::BadGrantTakenOnShape(kind, defect, verb));
                }
            }
        }
    }
    all
}

pub struct MoreRig {
    base: FakePublisherRig,
    flaw: More,
}

impl MoreRig {
    pub fn flawed(flaw: More) -> Self {
        Self {
            base: FakePublisherRig::conforming(),
            flaw,
        }
    }
}

pub struct Wrapped {
    base: Flawed,
    flaw: More,
    shape: ChannelShape,
    /// A commit was raced.
    raced: bool,
    /// The coordinates of every call that went through, before the call that is running.
    known: Vec<Coordinate>,
    committed: Vec<Coordinate>,
    staged: Vec<(Coordinate, Vec<u8>)>,
    aborted: Vec<Coordinate>,
}

impl PublisherRig for MoreRig {
    type Publisher = Wrapped;

    fn publisher(&self, shape: &ChannelShape) -> Wrapped {
        Wrapped {
            base: self.base.publisher(shape),
            flaw: self.flaw,
            shape: *shape,
            raced: false,
            known: Vec::new(),
            committed: Vec::new(),
            staged: Vec::new(),
            aborted: Vec::new(),
        }
    }

    fn arrange(&self, p: &mut Wrapped, what: &Arrangement) {
        if matches!(what, Arrangement::RaceNextCommit(_)) {
            p.raced = true;
        }
        if self.flaw == More::NonStagingSetCommitIsTorn
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
}

/// Whether `a` and `b` are two coordinates of one name that differ in `field` and in no other.
fn differs_only_in(a: &Coordinate, b: &Coordinate, field: Field) -> bool {
    let agree = |same: bool, of: Field| if field == of { !same } else { same };
    a.name == b.name
        && agree(a.channel == b.channel, Field::Channel)
        && agree(a.version == b.version, Field::Version)
        && agree(a.platform == b.platform, Field::Platform)
        && agree(a.r#type == b.r#type, Field::Type)
}

/// The coordinate a line of a grant names.
fn parse_key(line: &str) -> Option<Coordinate> {
    let parts: Vec<&str> = line.split('/').collect();
    let [channel, kind, name, version, platform] = parts.as_slice() else {
        return None;
    };
    Some(Coordinate {
        channel: (*channel).to_owned(),
        r#type: kind.parse().ok()?,
        name: (*name).to_owned(),
        version: (*version).to_owned(),
        platform: (*platform).to_owned(),
    })
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

fn lines(g: &Grant) -> Vec<String> {
    String::from_utf8_lossy(g.secret())
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Whether `g` is a stage or a commit grant.
fn is_writer(g: &Grant) -> bool {
    matches!(
        lines(g).first().map(String::as_str),
        Some("stage" | "commit")
    )
}

impl Wrapped {
    fn is(&self, flaw: More) -> bool {
        self.flaw == flaw
    }

    fn internal(&self, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        issue(
            &self.base.clock,
            &federated_repository(),
            phase,
            coordinates,
        )
    }

    fn on(&self, kind: Kind) -> bool {
        match kind {
            Kind::Private => self.shape.private && !self.shape.group_addressed && self.shape.stages,
            Kind::Grouped => self.shape.group_addressed,
            Kind::NonStaging => !self.shape.stages,
        }
    }

    /// In what way `g` is bad for `verb` over `coordinates`.
    fn defect(&self, g: &Grant, verb: Verb, coordinates: &[Coordinate]) -> Option<Defect> {
        let text = lines(g);
        let scope = text.first().map_or("", String::as_str);
        let repository = text.get(1).map_or("", String::as_str);
        let needed = match verb {
            Verb::Stage | Verb::Abort => Some("stage"),
            Verb::Commit | Verb::Yank => Some("commit"),
            Verb::Observe | Verb::Fetch => None,
        };
        if g.expires_ms()
            .is_some_and(|e| self.base.clock.now_millis() >= e)
        {
            return Some(Defect::Expired);
        }
        if repository != LOCATION {
            return Some(Defect::Foreign);
        }
        if let Some(needed) = needed
            && scope != needed
            && scope != "read"
        {
            return Some(Defect::OtherWritePhase);
        }
        let named = text.get(2..).unwrap_or_default();
        coordinates
            .iter()
            .any(|c| !named.contains(&coordinate_key(c)))
            .then_some(Defect::Uncovered)
    }

    /// Whether `g` names, for every coordinate of the call, one that differs from it in `field`
    /// only.
    fn names_a_sibling(g: &Grant, coordinates: &[Coordinate], field: Field) -> bool {
        let named: Vec<Coordinate> = lines(g)
            .iter()
            .skip(2)
            .filter_map(|l| parse_key(l))
            .collect();
        coordinates
            .iter()
            .all(|c| named.iter().any(|n| differs_only_in(n, c, field)))
    }

    /// The grant a lax stub takes `g` for, when it takes it for another: the refusal of `result`
    /// is one it lifts.
    fn lenient<T>(
        &self,
        verb: Verb,
        g: &Grant,
        coordinates: &[Coordinate],
        result: &Result<T, SeamError>,
    ) -> Option<Grant> {
        let Err(SeamError::GrantRefused(GrantFault::Denied | GrantFault::Expired)) = result else {
            return None;
        };
        if g.is_none() {
            return None;
        }
        let take = match self.flaw {
            More::GrantBlindToField(field, v) => {
                v == verb && Self::names_a_sibling(g, coordinates, field)
            }
            More::BadGrantTakenOnShape(kind, defect, v) => {
                v == verb && self.on(kind) && self.defect(g, verb, coordinates) == Some(defect)
            }
            _ => false,
        };
        let phase = match verb {
            Verb::Stage | Verb::Abort => GrantPhase::Stage,
            Verb::Commit | Verb::Yank => GrantPhase::Commit,
            Verb::Observe | Verb::Fetch => GrantPhase::Read,
        };
        take.then(|| self.internal(phase, coordinates))
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

    fn observe(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        set: &[StageItem<'_>],
    ) -> Result<Presence, SeamError> {
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
        if let More::ObserveStagedSibling(field) = self.flaw
            && matches!(result, Ok(Presence::Absent))
            && !g.is_none()
            && self
                .staged
                .iter()
                .any(|(c, _)| differs_only_in(c, k, field) && self.base.inner.is_staged(c))
        {
            result = Ok(Presence::Staged(Held::default()));
        }
        result
    }

    fn stage(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<StageAnswer, SeamError> {
        let coordinates: Vec<Coordinate> = items.iter().map(|i| i.coordinate.clone()).collect();
        let used = if self.is(More::DupStageDeduped) {
            deduped(items)
        } else {
            items.to_vec()
        };
        let mut result = self.base.stage(g, &used);
        let retry = self.lenient(Verb::Stage, g, &coordinates, &result);
        if let Some(grant) = retry {
            result = self.base.stage(&grant, &used);
        }
        if result == Ok(StageAnswer::Duplicate) && self.is(More::StagePartial) {
            for item in items {
                let one = [item.coordinate.clone()];
                let grant = self.internal(GrantPhase::Stage, &one);
                let _ = self.base.stage(&grant, std::slice::from_ref(item));
            }
        }
        if result == Ok(StageAnswer::Staged) {
            let taken = match self.flaw {
                More::StageAfterAbortIsDuplicate => {
                    coordinates.iter().any(|c| self.aborted.contains(c))
                }
                More::StageSiblingTaken(field) => coordinates
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
            && lines(g).first().map(String::as_str) == Some("commit")
        {
            retry = Some(self.internal(GrantPhase::Stage, ks));
        }
        if let Some(grant) = retry {
            result = self.base.abort(&grant, ks);
        }
        if result.is_ok() {
            self.aborted.extend(ks.iter().cloned());
            if let More::AbortSiblings(field) = self.flaw {
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
        let used = if self.is(More::DupCommitDeduped) {
            deduped(items)
        } else {
            items.to_vec()
        };
        let yanked_first = items
            .first()
            .is_some_and(|item| self.base.inner.is_yanked(item.coordinate));
        let mut result = self.base.commit(g, &used);
        let retry = self.lenient(Verb::Commit, g, &coordinates, &result);
        if let Some(grant) = retry {
            result = self.base.commit(&grant, &used);
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
        }
        if !self.shape.stages
            && self.is(More::NonStagingTransientIsPermanent)
            && let Err(SeamError::Transient(_)) = result
        {
            result = Err(SeamError::Permanent("the commit failed".to_owned()));
        }
        if result == Ok(CommitAnswer::Committed) {
            let taken = match self.flaw {
                More::CommitSiblingTaken(field) => coordinates.iter().any(|c| {
                    self.committed
                        .iter()
                        .any(|held| differs_only_in(held, c, field))
                }),
                _ => false,
            };
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
        if let (Ok(()), More::YankSiblings(field)) = (&result, self.flaw) {
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
        let mut result = self.base.fetch(g, k);
        let retry = self.lenient(Verb::Fetch, g, std::slice::from_ref(k), &result);
        if let Some(grant) = retry {
            result = self.base.fetch(&grant, k);
        }
        if self.is(More::FetchWithWriterGrantSeesStaged)
            && matches!(result, Ok(None))
            && is_writer(g)
            && self.base.inner.is_staged(k)
            && let Some((_, bytes)) = self.staged.iter().rev().find(|(c, _)| c == k)
        {
            result = Ok(Some(bytes.clone()));
        }
        result
    }

    fn newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError> {
        let mut found = self.base.newest(g, ch, name)?;
        if self.is(More::NewestWithWriterGrantIncludesStaged) && is_writer(g) {
            for (c, _) in self.staged.clone() {
                if c.channel == ch
                    && c.name == name
                    && self.base.inner.is_staged(&c)
                    && let Some(held) = self.base.held_now(&c)
                {
                    found.push((c, held));
                }
            }
        }
        Ok(found)
    }

    fn list(&mut self, g: &Grant, scope: &ListScope) -> Result<Vec<Listed>, SeamError> {
        let mut listed = self.base.list(g, scope)?;
        if self.is(More::ListWithWriterGrantIncludesStaged) && is_writer(g) {
            for (c, _) in self.staged.clone() {
                if c.channel == scope.channel
                    && c.name.starts_with(&scope.prefix)
                    && self.base.inner.is_staged(&c)
                    && let Some(held) = self.base.held_now(&c)
                {
                    listed.push(Listed {
                        coordinate: c,
                        held,
                    });
                }
            }
        }
        Ok(listed)
    }
}
