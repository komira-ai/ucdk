// SPDX-License-Identifier: Apache-2.0
//! The stubs of a registry that is lax about a grant on one verb, in one field of a coordinate or
//! on one shape of channel: it takes a grant the conforming fake refuses for one it accepts, and
//! is the conforming fake otherwise.

use ucdk_core::Clock;
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::{Held, Presence};
use ucdk_release_fake::coordinate_key;
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::kit::LOCATION;
use ucdk_release_seams::{Grant, ListScope, Listed, Publisher, SeamError, StageItem};

use super::publisher_more::{Field, More, Wrapped, differs_only_in, is_writer};
use super::secret::texts;

/// A call of a registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Stage,
    Commit,
    Abort,
    Yank,
    Observe,
    Fetch,
    Newest,
    List,
}

impl Verb {
    /// Every verb.
    pub const ALL: [Self; 8] = [
        Self::Stage,
        Self::Commit,
        Self::Abort,
        Self::Yank,
        Self::Observe,
        Self::Fetch,
        Self::Newest,
        Self::List,
    ];

    /// The verbs whose call names a coordinate, which a grant may name or leave out.
    pub const NAMING: [Self; 6] = [
        Self::Stage,
        Self::Commit,
        Self::Abort,
        Self::Yank,
        Self::Observe,
        Self::Fetch,
    ];

    fn writes(self) -> bool {
        matches!(self, Self::Stage | Self::Commit | Self::Abort | Self::Yank)
    }
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

/// Which two of the three coordinates of a call a stub checks a grant for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pair {
    FirstAndLast,
    FirstAndMiddle,
    MiddleAndLast,
}

impl Pair {
    pub const ALL: [Self; 3] = [
        Self::FirstAndLast,
        Self::FirstAndMiddle,
        Self::MiddleAndLast,
    ];

    /// The positions in the call of the two coordinates.
    fn at(self) -> (usize, usize) {
        match self {
            Self::FirstAndLast => (0, 2),
            Self::FirstAndMiddle => (0, 1),
            Self::MiddleAndLast => (1, 2),
        }
    }
}

/// The shape of channel a stub is lax on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A public channel that stages and addresses staged state by coordinate.
    Public,
    /// A private channel that stages and addresses staged state by coordinate.
    Private,
    /// A channel that addresses staged state by group.
    Grouped,
    /// A channel that does not stage.
    NonStaging,
}

/// The phase of the grant a stub answers in a way the conforming fake does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holder {
    Stage,
    Commit,
    Read,
}

impl Holder {
    pub const ALL: [Self; 3] = [Self::Stage, Self::Commit, Self::Read];

    /// The phase of `g`, when it is a grant.
    fn of(g: &Grant) -> Option<Self> {
        match texts(g.secret()).first().map(String::as_str) {
            Some("stage") => Some(Self::Stage),
            Some("commit") => Some(Self::Commit),
            Some("read") => Some(Self::Read),
            _ => None,
        }
    }
}

/// How a reader asks for what another writer staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// An `observe` told the declared set.
    ObserveTold,
    /// An `observe` told no set.
    ObserveAlone,
    Fetch,
    Newest,
    List,
}

impl Seen {
    pub const ALL: [Self; 5] = [
        Self::ObserveTold,
        Self::ObserveAlone,
        Self::Fetch,
        Self::Newest,
        Self::List,
    ];
}

/// A stub that is lax in one way on one kind of channel, and is the conforming fake otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lax {
    /// On a channel of the kind, a read that is asked in the way, with a grant of the phase,
    /// shows the staged state of another writer.
    OthersStagedSeen(Kind, Seen, Holder),
    /// On a channel of the kind, an observe with a grant of the phase, told the declared set or
    /// not, answers `Absent` for a coordinate another writer committed.
    OthersCommittedBlind(Kind, Holder, bool),
    /// A fetch with a read grant of a coordinate that is held finds nothing.
    FetchFindsNothing,
    /// A fetch with a read grant of a coordinate that is held answers the bytes of another
    /// coordinate that is held.
    FetchAnswersAnother,
    /// A stage, a commit or an abort whose grant names some of the coordinates of the call and
    /// not all of them is taken for one that names them all.
    PartialCoverage(Kind, Verb),
    /// A read, an `observe`, a `fetch`, a `newest` or a `list`, with a stage or a commit grant
    /// that is bad in the way is taken for one with a good grant.
    WriterGrantOnRead(Kind, Defect, Verb),
    /// An observe by a stranger of a coordinate that is staged answers `Staged`.
    StrangerSeesStaged(Kind),
    /// An observe by a stranger of a coordinate another writer committed answers `Present`.
    StrangerSeesOthers(Kind),
}

/// Every stub that takes a grant for some of the coordinates of a call for a grant for all of them.
pub fn partial_coverage() -> Vec<More> {
    let mut all = Vec::new();
    for kind in [Kind::Public, Kind::Private, Kind::Grouped, Kind::NonStaging] {
        for verb in [Verb::Stage, Verb::Commit, Verb::Abort] {
            // A channel that does not stage is never asked to stage or to abort.
            if kind != Kind::NonStaging || verb == Verb::Commit {
                all.push(More::Lax(Lax::PartialCoverage(kind, verb)));
            }
        }
    }
    all
}

/// Every stub that takes a bad stage or commit grant for a good one on a read of a private
/// channel, on a kind of channel.
pub fn writer_grants_on_reads() -> Vec<More> {
    let mut all = Vec::new();
    for kind in [Kind::Private, Kind::Grouped, Kind::NonStaging] {
        for defect in [Defect::Foreign, Defect::Expired, Defect::Uncovered] {
            for verb in [Verb::Observe, Verb::Fetch, Verb::Newest, Verb::List] {
                // `newest` and `list` name no coordinate for a grant to leave out.
                if defect != Defect::Uncovered || matches!(verb, Verb::Observe | Verb::Fetch) {
                    all.push(More::Lax(Lax::WriterGrantOnRead(kind, defect, verb)));
                }
            }
        }
    }
    all
}

/// Every stub that lets a stranger observe what a private channel holds or has staged.
pub fn strangers_let_in() -> Vec<More> {
    vec![
        More::Lax(Lax::StrangerSeesStaged(Kind::Private)),
        More::Lax(Lax::StrangerSeesStaged(Kind::Grouped)),
        More::Lax(Lax::StrangerSeesOthers(Kind::Private)),
        More::Lax(Lax::StrangerSeesOthers(Kind::Grouped)),
        More::Lax(Lax::StrangerSeesOthers(Kind::NonStaging)),
    ]
}

/// Every stub that shows the staged state of another writer, to a reader of each phase that asks
/// in each way, on each kind of channel that stages.
pub fn others_staged_seen() -> Vec<More> {
    let mut all = Vec::new();
    for kind in [Kind::Public, Kind::Private, Kind::Grouped] {
        for seen in Seen::ALL {
            for holder in Holder::ALL {
                all.push(More::Lax(Lax::OthersStagedSeen(kind, seen, holder)));
            }
        }
    }
    all
}

/// Every stub that answers `Absent` to an observe of a coordinate another writer committed, with a
/// grant of each phase the channel has, told the set and not, on each kind of channel.
pub fn others_committed_blind() -> Vec<More> {
    let mut all = Vec::new();
    for kind in [Kind::Public, Kind::Private, Kind::Grouped, Kind::NonStaging] {
        for holder in Holder::ALL {
            // A channel that does not stage has no stage grant.
            if kind == Kind::NonStaging && holder == Holder::Stage {
                continue;
            }
            for told in [false, true] {
                all.push(More::Lax(Lax::OthersCommittedBlind(kind, holder, told)));
            }
        }
    }
    all
}

/// Every stub that takes a grant naming a coordinate that differs in one field.
pub fn field_blind() -> Vec<More> {
    let mut all = Vec::new();
    for field in Field::ALL {
        for verb in Verb::NAMING {
            all.push(More::GrantBlindToField(field, verb));
        }
    }
    all
}

/// Every stub that is lax about a bad grant on a shape of channel other than the public one that
/// stages.
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
                // A read has no phase to get wrong, and `newest` and `list` name no coordinate
                // for a grant to leave out; a channel that does not stage is never asked to
                // stage or abort; the reads of a private plain channel are held by the stubs
                // that take a read grant for another.
                let names_nothing = matches!(verb, Verb::Newest | Verb::List);
                let kept = match kind {
                    // The public channel that stages is held by the stubs of every channel.
                    Kind::Public => false,
                    Kind::Private => verb.writes(),
                    Kind::Grouped => {
                        verb.writes()
                            || (defect != Defect::OtherWritePhase
                                && !(names_nothing && defect == Defect::Uncovered))
                    }
                    Kind::NonStaging => {
                        matches!(verb, Verb::Commit | Verb::Yank)
                            || (!verb.writes()
                                && defect != Defect::OtherWritePhase
                                && !(names_nothing && defect == Defect::Uncovered))
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

/// Every stub that checks a grant for two of the three coordinates of a stage, a commit or an
/// abort and takes a grant that names those two for one that names the three.
pub fn coverage_of_two() -> Vec<More> {
    let mut all = Vec::new();
    for verb in [Verb::Stage, Verb::Commit, Verb::Abort] {
        for pair in Pair::ALL {
            all.push(More::CoverageOfTwo(verb, pair));
        }
    }
    all
}

/// Every stub that takes a read grant for the grant of a write, on a kind of channel.
pub fn read_grant_writes() -> Vec<More> {
    let mut all = Vec::new();
    for kind in [Kind::Public, Kind::Private, Kind::Grouped, Kind::NonStaging] {
        for verb in Verb::ALL.into_iter().filter(|v| v.writes()) {
            // A channel that does not stage is never asked to stage or to abort.
            if kind != Kind::NonStaging || matches!(verb, Verb::Commit | Verb::Yank) {
                all.push(More::ReadGrantWrites(kind, verb));
            }
        }
    }
    all
}

/// Every stub that shows staged state to a stage or a commit grant that reads with `fetch`,
/// `newest` or `list`, on a private channel and on one addressed by group.
pub fn writers_see_staged() -> Vec<More> {
    let mut all = Vec::new();
    for kind in [Kind::Private, Kind::Grouped] {
        for verb in [Verb::Fetch, Verb::Newest, Verb::List] {
            all.push(More::WriterSeesStaged(kind, verb));
        }
    }
    all
}

/// Every stub that shows the staged state of a writer to an observe with a read grant, on a
/// kind of channel.
pub fn read_grants_see_staged() -> Vec<More> {
    [Kind::Public, Kind::Private, Kind::Grouped]
        .into_iter()
        .map(More::ReadSeesStaged)
        .collect()
}

/// The coordinate a text of a grant names.
fn parse_key(key: &str) -> Option<Coordinate> {
    let [channel, kind, name, version, platform] = texts(key.as_bytes()).try_into().ok()?;
    Some(Coordinate {
        channel,
        r#type: kind.parse().ok()?,
        name,
        version,
        platform,
    })
}

impl Wrapped {
    pub(super) fn on(&self, kind: Kind) -> bool {
        match kind {
            Kind::Public => self.shape.stages && !self.shape.private && !self.shape.group_addressed,
            Kind::Private => self.shape.private && !self.shape.group_addressed && self.shape.stages,
            Kind::Grouped => self.shape.group_addressed,
            Kind::NonStaging => !self.shape.stages,
        }
    }

    /// In what way `g` is bad for `verb` over `coordinates`.
    fn defect(&self, g: &Grant, verb: Verb, coordinates: &[Coordinate]) -> Option<Defect> {
        let text = texts(g.secret());
        let scope = text.first().map_or("", String::as_str);
        let repository = text.get(1).map_or("", String::as_str);
        let needed = match verb {
            Verb::Stage | Verb::Abort => Some("stage"),
            Verb::Commit | Verb::Yank => Some("commit"),
            Verb::Observe | Verb::Fetch | Verb::Newest | Verb::List => None,
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
        let named: Vec<Coordinate> = texts(g.secret())
            .iter()
            .skip(2)
            .filter_map(|text| parse_key(text))
            .collect();
        coordinates
            .iter()
            .all(|c| named.iter().any(|n| differs_only_in(n, c, field)))
    }

    /// Whether `g` names the two coordinates of the call that `pair` stands for.
    fn names_a_pair(g: &Grant, coordinates: &[Coordinate], pair: Pair) -> bool {
        let named = texts(g.secret());
        let (first, second) = pair.at();
        [first, second].iter().all(|at| {
            coordinates
                .get(*at)
                .is_some_and(|c| named.iter().skip(2).any(|text| *text == coordinate_key(c)))
        })
    }

    /// Whether the stub `lax` takes the grant `g` for a good one on `verb` over `coordinates`.
    fn lax_takes(&self, lax: Lax, verb: Verb, g: &Grant, coordinates: &[Coordinate]) -> bool {
        match lax {
            Lax::PartialCoverage(kind, v) => {
                let named = texts(g.secret());
                v == verb
                    && self.on(kind)
                    && self.defect(g, verb, coordinates) == Some(Defect::Uncovered)
                    && coordinates
                        .iter()
                        .any(|c| named.iter().skip(2).any(|text| *text == coordinate_key(c)))
            }
            Lax::WriterGrantOnRead(kind, defect, v) => {
                v == verb
                    && self.on(kind)
                    && is_writer(g)
                    && self.defect(g, verb, coordinates) == Some(defect)
            }
            Lax::StrangerSeesStaged(_)
            | Lax::StrangerSeesOthers(_)
            | Lax::OthersStagedSeen(..)
            | Lax::OthersCommittedBlind(..)
            | Lax::FetchFindsNothing
            | Lax::FetchAnswersAnother => false,
        }
    }

    /// Whether the stub is of `kind`, shows the staged state of another writer to a read asked
    /// in the way `seen`, to a grant `g` of the phase it names, and `k` is staged by another.
    fn shows_others_staged(&self, seen: Seen, g: &Grant, k: Option<&Coordinate>) -> bool {
        match self.flaw() {
            Some(More::Lax(Lax::OthersStagedSeen(kind, s, holder))) => {
                s == seen
                    && self.on(kind)
                    && Holder::of(g) == Some(holder)
                    && k.is_none_or(|k| self.staged_by_others.iter().any(|(c, _)| c == k))
            }
            _ => false,
        }
    }

    /// What an observe by `g` of `k` told `set` answers when `result` is what the conforming fake
    /// answered: a stub that lets a stranger in answers it as a grant would, one that shows the
    /// staged state of another writer shows it, and one that is blind to what another writer
    /// committed does not see it.
    pub(super) fn lax_observe(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        set: &[StageItem<'_>],
        result: Result<Presence, SeamError>,
    ) -> Result<Presence, SeamError> {
        let seen = if set.is_empty() {
            Seen::ObserveAlone
        } else {
            Seen::ObserveTold
        };
        if matches!(result, Ok(Presence::Absent)) && self.shows_others_staged(seen, g, Some(k)) {
            return Ok(Presence::Staged(Held::default()));
        }
        if let (Some(More::Lax(Lax::OthersCommittedBlind(kind, holder, told))), Ok(_)) =
            (self.flaw(), &result)
            && self.on(kind)
            && Holder::of(g) == Some(holder)
            && told != set.is_empty()
            && self.others.contains(k)
        {
            return Ok(Presence::Absent);
        }
        if !g.is_none() || !matches!(result, Err(SeamError::GrantRefused(GrantFault::Denied))) {
            return result;
        }
        match self.flaw() {
            Some(More::Lax(Lax::StrangerSeesStaged(kind)))
                if self.on(kind)
                    && self.staged.iter().any(|(c, _)| c == k)
                    && self.base.inner.is_staged(k) =>
            {
                Ok(Presence::Staged(Held::default()))
            }
            Some(More::Lax(Lax::StrangerSeesOthers(kind)))
                if self.on(kind) && self.others.contains(k) =>
            {
                let grant = self.internal(GrantPhase::Read, std::slice::from_ref(k));
                self.base.observe(&grant, k, set)
            }
            _ => result,
        }
    }

    /// Whether a stub shows a coordinate staged to `verb` when the grant is a stage or a commit
    /// grant.
    pub(super) fn writer_sees_staged(&self, verb: Verb) -> bool {
        match self.flaw() {
            Some(More::WriterSeesStaged(kind, v)) => v == verb && self.on(kind),
            _ => false,
        }
    }

    /// Whether a stub shows `k` staged to an observe with the read grant `g`, told a set or not.
    pub(super) fn read_grant_sees_staged(&self, g: &Grant, k: &Coordinate, told: bool) -> bool {
        if texts(g.secret()).first().map(String::as_str) != Some("read") {
            return false;
        }
        match self.flaw() {
            Some(More::ReadSeesStaged(kind)) => {
                self.on(kind)
                    && self.staged.iter().any(|(c, _)| c == k)
                    && self.base.inner.is_staged(k)
                    && (told || !self.shape.group_addressed)
            }
            Some(More::ReadSeesOthersStaged) => self.staged_by_others.iter().any(|(c, _)| c == k),
            _ => false,
        }
    }

    /// What a fetch by `g` of `k` answers when `result` is what the conforming fake answered: a
    /// stub that is wrong about a read grant's fetch of what is held answers it wrongly, and one
    /// that shows the staged state of another writer answers its bytes.
    pub(super) fn lax_fetch(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        result: Result<Option<Vec<u8>>, SeamError>,
    ) -> Result<Option<Vec<u8>>, SeamError> {
        let read = Holder::of(g) == Some(Holder::Read);
        match (self.flaw(), result) {
            (Some(More::Lax(Lax::FetchFindsNothing)), Ok(Some(_))) if read => Ok(None),
            (Some(More::Lax(Lax::FetchAnswersAnother)), Ok(Some(held))) if read => {
                let other = self
                    .committed
                    .iter()
                    .find(|c| *c != k)
                    .and_then(|c| self.base.inner.bytes(c));
                Ok(Some(other.map_or(held, <[u8]>::to_vec)))
            }
            (_, Ok(None)) if self.shows_others_staged(Seen::Fetch, g, Some(k)) => {
                let theirs = self.staged_by_others.iter().find(|(c, _)| c == k);
                Ok(theirs.map(|(_, bytes)| bytes.clone()))
            }
            (_, result) => result,
        }
    }

    /// What a `newest` by `g` answers when `found` is what the conforming fake answered.
    pub(super) fn lax_newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
        mut found: Vec<(Coordinate, Held)>,
    ) -> Vec<(Coordinate, Held)> {
        if self.shows_others_staged(Seen::Newest, g, None) {
            for (c, _) in &self.staged_by_others {
                if c.channel == ch && c.name == name {
                    found.push((c.clone(), Held::default()));
                }
            }
        }
        found
    }

    /// What a `list` by `g` of `scope` answers when `listed` is what the conforming fake answered.
    pub(super) fn lax_list(
        &mut self,
        g: &Grant,
        scope: &ListScope,
        mut listed: Vec<Listed>,
    ) -> Vec<Listed> {
        if self.shows_others_staged(Seen::List, g, None) {
            for (c, _) in &self.staged_by_others {
                if c.channel == scope.channel && c.name.starts_with(&scope.prefix) {
                    listed.push(Listed {
                        coordinate: c.clone(),
                        held: Held::default(),
                    });
                }
            }
        }
        listed
    }

    /// The grant a lax stub takes `g` for, when it takes it for another: the refusal of `result`
    /// is one it lifts.
    pub(super) fn lenient<T>(
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
        let take = match self.flaw() {
            Some(More::GrantBlindToField(field, v)) => {
                v == verb && Self::names_a_sibling(g, coordinates, field)
            }
            Some(More::BadGrantTakenOnShape(kind, defect, v)) => {
                v == verb && self.on(kind) && self.defect(g, verb, coordinates) == Some(defect)
            }
            Some(More::Lax(lax)) => self.lax_takes(lax, verb, g, coordinates),
            Some(More::CoverageOfTwo(v, pair)) => {
                v == verb
                    && self.defect(g, verb, coordinates) == Some(Defect::Uncovered)
                    && Self::names_a_pair(g, coordinates, pair)
            }
            Some(More::ReadGrantWrites(kind, v)) => {
                v == verb
                    && self.on(kind)
                    && texts(g.secret()).first().map(String::as_str) == Some("read")
                    && self.defect(g, verb, coordinates).is_none()
            }
            _ => false,
        };
        let phase = match verb {
            Verb::Stage | Verb::Abort => GrantPhase::Stage,
            Verb::Commit | Verb::Yank => GrantPhase::Commit,
            Verb::Observe | Verb::Fetch | Verb::Newest | Verb::List => GrantPhase::Read,
        };
        take.then(|| self.internal(phase, coordinates))
    }
}
