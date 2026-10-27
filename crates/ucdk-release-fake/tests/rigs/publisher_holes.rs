// SPDX-License-Identifier: Apache-2.0
//! Stubs of a registry that gives away more than it should of what it holds, or reads more than it
//! says it does: the conforming fake wrapped so that it is wrong about one read in one way. A
//! stranger told the declared set sees staged state; a private channel refuses a stranger or a bad
//! grant only for what it holds and answers an empty answer for what it does not, so that whoever
//! asks finds out which packages exist; an observe told no set reads the group.

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::{ArtifactType, Coordinate};
use ucdk_release_seams::kit::{
    Arrangement, ChannelShape, Payload, PublisherRig, ReadStep, Sibling,
};
use ucdk_release_seams::{
    CommitAnswer, CommitUnit, Grant, ListScope, Listed, Publisher, SeamError, StageAnswer,
    StageItem, YankUnit,
};

use super::publisher::{FakePublisherRig, Flawed};
use super::publisher_lax::{Kind, Verb};

/// What a stub gets wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hole {
    /// On a channel of the kind, a stranger who is told the declared set sees the state the caller
    /// staged: the answer is `Staged`, and not `Absent`.
    StrangerToldTheSetSeesStaged(Kind),
    /// On a private channel of the kind, the read the verb names, refused to a stranger or for a
    /// bad grant, is answered with nothing when the channel holds nothing of what it asks about:
    /// `Absent`, no bytes, no coordinates. What is held is refused with `Denied`.
    RefusesOnlyWhatIsHeld(Kind, Verb),
    /// An observe told no set reads the group as well as the index, on a channel addressed by group.
    EmptySetReadsTheGroup,
}

/// Whether a channel of `shape` is of `kind`.
fn is_of(kind: Kind, shape: &ChannelShape) -> bool {
    match kind {
        Kind::Public => shape.stages && !shape.private && !shape.group_addressed,
        Kind::Private => shape.private && !shape.group_addressed && shape.stages,
        Kind::Grouped => shape.group_addressed,
        Kind::NonStaging => !shape.stages,
    }
}

/// Every stub that refuses only what a private channel holds, for each kind of private channel and
/// each read.
pub fn refusing_only_what_is_held() -> Vec<Hole> {
    let mut all = Vec::new();
    for kind in [Kind::Private, Kind::Grouped, Kind::NonStaging] {
        for verb in [Verb::Observe, Verb::Fetch, Verb::Newest, Verb::List] {
            all.push(Hole::RefusesOnlyWhatIsHeld(kind, verb));
        }
    }
    all
}

/// The rig of the fake with a stub behind its channels.
pub struct HoleRig {
    base: FakePublisherRig,
    hole: Hole,
}

impl HoleRig {
    pub fn new(hole: Hole) -> Self {
        Self::on(FakePublisherRig::conforming(), hole)
    }

    /// The stub of `hole` around the rig `base`.
    pub fn on(base: FakePublisherRig, hole: Hole) -> Self {
        Self { base, hole }
    }
}

pub struct Holed {
    base: Flawed,
    hole: Hole,
    shape: ChannelShape,
    /// What the channel holds, committed, staged or staged by another writer.
    held: Vec<Coordinate>,
    /// What the caller staged.
    staged: Vec<Coordinate>,
    /// Where in the registry's log of reads the index read of each observe told no set is.
    without_a_set: Vec<usize>,
}

impl Holed {
    fn is(&self, hole: Hole) -> bool {
        self.hole == hole
    }

    /// Whether the stub answers an empty answer to a read of `verb` that was refused.
    fn answers_nothing(&self, verb: Verb) -> bool {
        matches!(self.hole, Hole::RefusesOnlyWhatIsHeld(kind, v)
            if v == verb && self.shape.private && is_of(kind, &self.shape))
    }

    fn refused<T>(result: &Result<T, SeamError>) -> bool {
        matches!(
            result,
            Err(SeamError::GrantRefused(
                GrantFault::Denied | GrantFault::Expired
            ))
        )
    }
}

impl PublisherRig for HoleRig {
    type Publisher = Holed;

    fn shapes(&self) -> Vec<ChannelShape> {
        self.base.shapes()
    }

    fn publisher(&self, shape: &ChannelShape) -> Holed {
        Holed {
            base: self.base.publisher(shape),
            hole: self.hole,
            shape: *shape,
            held: Vec::new(),
            staged: Vec::new(),
            without_a_set: Vec::new(),
        }
    }

    fn arrange(&self, p: &mut Holed, what: &Arrangement) {
        match what {
            Arrangement::HeldByAnother { coordinate, .. }
            | Arrangement::StagedByAnother { coordinate, .. } => p.held.push(coordinate.clone()),
            _ => {}
        }
        self.base.arrange(&mut p.base, what);
    }

    fn grant(&self, p: &Holed, phase: GrantPhase, ks: &[Coordinate]) -> Grant {
        self.base.grant(&p.base, phase, ks)
    }

    fn foreign_grant(&self, p: &Holed, phase: GrantPhase, ks: &[Coordinate]) -> Grant {
        self.base.foreign_grant(&p.base, phase, ks)
    }

    fn advance(&self, ms: u64) {
        self.base.advance(ms);
    }

    fn sent(&self, p: &Holed) -> usize {
        self.base.sent(&p.base)
    }

    fn reads(&self, p: &Holed) -> Vec<ReadStep> {
        let mut all = Vec::new();
        for (at, step) in self.base.reads(&p.base).into_iter().enumerate() {
            if p.without_a_set.contains(&at) {
                all.push(ReadStep::Group);
            }
            all.push(step);
        }
        all
    }

    fn coordinate(&self, label: &str) -> Coordinate {
        self.base.coordinate(label)
    }

    fn payload(&self, k: &Coordinate, seed: u8) -> Payload {
        self.base.payload(k, seed)
    }

    fn siblings(&self, base: &Coordinate) -> Vec<Sibling> {
        self.base.siblings(base)
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

impl Publisher for Holed {
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
        let before = self.base.inner.read_log().len();
        let result = self.base.observe(g, k, set);
        if set.is_empty()
            && self.shape.group_addressed
            && self.is(Hole::EmptySetReadsTheGroup)
            && self.base.inner.read_log().len() > before
        {
            self.without_a_set.push(before);
        }
        if let Hole::StrangerToldTheSetSeesStaged(kind) = self.hole
            && is_of(kind, &self.shape)
            && g.is_none()
            && !set.is_empty()
            && matches!(result, Ok(Presence::Absent))
            && self.staged.contains(k)
            && self.base.inner.is_staged(k)
        {
            return Ok(Presence::Staged(Held::default()));
        }
        if Self::refused(&result) && self.answers_nothing(Verb::Observe) && !self.held.contains(k) {
            return Ok(Presence::Absent);
        }
        result
    }

    fn stage(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<StageAnswer, SeamError> {
        let result = self.base.stage(g, items);
        if result == Ok(StageAnswer::Staged) {
            let ks: Vec<Coordinate> = items.iter().map(|i| i.coordinate.clone()).collect();
            self.held.extend(ks.iter().cloned());
            self.staged.extend(ks);
        }
        result
    }

    fn abort(&mut self, g: &Grant, ks: &[Coordinate]) -> Result<(), SeamError> {
        self.base.abort(g, ks)
    }

    fn commit(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<CommitAnswer, SeamError> {
        let result = self.base.commit(g, items);
        if result == Ok(CommitAnswer::Committed) {
            self.held
                .extend(items.iter().map(|item| item.coordinate.clone()));
        }
        result
    }

    fn yank(&mut self, g: &Grant, k: &Coordinate, reason: &str) -> Result<(), SeamError> {
        self.base.yank(g, k, reason)
    }

    fn fetch(&mut self, g: &Grant, k: &Coordinate) -> Result<Option<Vec<u8>>, SeamError> {
        let result = self.base.fetch(g, k);
        if Self::refused(&result) && self.answers_nothing(Verb::Fetch) && !self.held.contains(k) {
            return Ok(None);
        }
        result
    }

    fn newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError> {
        let result = self.base.newest(g, ch, name);
        let holds = self.held.iter().any(|c| c.channel == ch && c.name == name);
        if Self::refused(&result) && self.answers_nothing(Verb::Newest) && !holds {
            return Ok(Vec::new());
        }
        result
    }

    fn list(&mut self, g: &Grant, scope: &ListScope) -> Result<Vec<Listed>, SeamError> {
        let result = self.base.list(g, scope);
        let holds = self
            .held
            .iter()
            .any(|c| c.channel == scope.channel && c.name.starts_with(&scope.prefix));
        if Self::refused(&result) && self.answers_nothing(Verb::List) && !holds {
            return Ok(Vec::new());
        }
        result
    }
}
