// SPDX-License-Identifier: Apache-2.0
//! The publisher rig: the fake registry in each shape, the grants its issuer gives, and the
//! stubs that break one promise of a registry.

use std::time::Duration;

use ucdk_core::ManualClock;
use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_fake::{FakeGrantSource, FakeRegistry, JobClaims, Read, coordinate_key};
use ucdk_release_proto::v1::{ArtifactType, Coordinate, Repository};
use ucdk_release_seams::kit::{
    Arrangement, ChannelShape, LOCATION, PublisherRig, ReadStep, federated_repository, matching_job,
};
use ucdk_release_seams::{
    CommitAnswer, CommitUnit, Grant, GrantRequest, GrantSource, ListScope, Listed, Publisher,
    SeamError, StageAnswer, StageItem,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// A coordinate nobody wrote is an error to observe.
    AbsentIsAnError,
    /// A staged coordinate is observed with a wrong digest.
    StagedDigestIsWrong,
    /// A stranger sees a staged coordinate.
    StrangerSeesStaged,
    /// A stage grant cannot see what it staged.
    StageGrantIsBlindToStaged,
    /// A commit grant cannot see staged state.
    CommitGrantIsBlindToStaged,
    /// A committed coordinate is observed with a wrong digest.
    PresentDigestIsWrong,
    /// A stage over the same bytes is answered `Staged`.
    StageOverTheSameBytesIsStaged,
    /// A commit over a coordinate that was not staged and holds other bytes replaces them.
    CommitOverOtherBytesOverwrites,
    /// An abort does nothing.
    AbortDoesNothing,
    /// A commit of a set that fails leaves the first coordinate readable.
    SetCommitIsTorn,
    /// A channel that does not stage says it does.
    StagesLies,
    /// A coordinate of another writer is observed without its digest.
    OthersHaveNoDigest,
    /// A yanked coordinate cannot be fetched.
    YankedIsNotFetchable,
    /// A commit over a yanked coordinate answers `Committed`.
    CommitOverYankedCommits,
    /// After an answer was lost, an observe answers `Absent`.
    ObserveAfterALostAnswerIsAbsent,
    /// While the listing lags, an observe reads the listing.
    ObserveReadsTheLaggingList,
    /// A read without a grant of a public channel is denied, whatever it reads but an observe.
    AnonymousFetchIsDenied,
    /// A grant for another repository is taken for this one.
    AForeignGrantIsAccepted,
    /// A grant of the other write phase is taken for the phase of the call.
    TheWrongPhaseIsAccepted,
    /// An expired grant is taken for a good one.
    AnExpiredGrantIsAccepted,
    /// `newest` leaves out a yanked coordinate.
    NewestDropsYanked,
    /// A committed coordinate is observed without its stamp.
    TheStampIsDropped,
    /// A grant on a plain channel does not see what another writer committed.
    GrantedObserveIsBlindToOthers,
    /// The group and the index are read in the other order.
    IndexIsReadBeforeTheGroup,
    /// A grant on a grouped channel does not see what another writer committed.
    GroupedObserveIsBlindToOthers,
    /// A failed group read is taken for no group.
    FailedGroupReadIsAbsent,
    /// The declared set is dropped from an observe of staged state.
    TheDeclaredSetIsDropped,
    /// An observe with no set sees staged state on a grouped channel.
    AnEmptySetSeesStagedState,
    /// A read grant may stage, commit, abort and yank.
    AReadGrantMayWrite,
    /// A commit that lost a race replaces the bytes that won it.
    ARacedCommitOverwrites,
    /// A failure after a commit that landed is `Permanent`.
    TransientAfterCommitIsPermanent,
    /// A stage over the bytes of another writer is answered `Staged`.
    StageOverOtherBytesIsStaged,
    /// A stranger's `observe` of a private channel is answered as a read grant's would be.
    StrangerObservesPrivate,
    /// A stranger's `fetch` of a private channel is answered as a read grant's would be.
    StrangerFetchesPrivate,
    /// A stranger's `newest` of a private channel is answered as a read grant's would be.
    StrangerNewestPrivate,
    /// A stranger's `list` of a private channel is answered as a read grant's would be.
    StrangerListsPrivate,
    /// A stranger fetches the bytes of a staged coordinate.
    StrangerFetchesStaged,
    /// `list` names staged coordinates.
    ListIncludesStaged,
    /// `newest` names staged coordinates.
    NewestIncludesStaged,
    /// `newest` answers every coordinate whose name starts with the name asked for.
    NewestMatchesByPrefix,
    /// `newest` answers the coordinates of the name in every channel.
    NewestIgnoresTheChannel,
    /// An abort by a stage grant gives back every staged coordinate, not only those it names.
    AbortGivesBackAllStaged,
    /// A yank is taken from a stage grant.
    YankIgnoresThePhase,
    /// A stage grant that does not cover every coordinate of the call is taken for one that does.
    StageIgnoresCoverage,
    /// A commit grant that does not cover every coordinate of the call is taken for one that
    /// does.
    CommitIgnoresCoverage,
}

pub struct FakePublisherRig {
    clock: ManualClock,
    flaw: Option<Flaw>,
}

impl FakePublisherRig {
    pub fn conforming() -> Self {
        Self {
            clock: ManualClock::at(1_000_000),
            flaw: None,
        }
    }

    pub fn flawed(flaw: Flaw) -> Self {
        Self {
            flaw: Some(flaw),
            ..Self::conforming()
        }
    }

    fn issue(
        &self,
        repository: &Repository,
        phase: GrantPhase,
        coordinates: &[Coordinate],
    ) -> Grant {
        issue(&self.clock, repository, phase, coordinates)
    }
}

/// A grant of `phase` for `coordinates` in `repository`, good now.
fn issue(
    clock: &ManualClock,
    repository: &Repository,
    phase: GrantPhase,
    coordinates: &[Coordinate],
) -> Grant {
    let job = matching_job();
    let claims = JobClaims {
        subject: job.subject,
        audience: job.audience,
        environment: job.environment,
        pinned: job.pinned,
    };
    let mut source = FakeGrantSource::new(clock, claims);
    source
        .grant(&GrantRequest {
            repository,
            phase,
            coordinates,
            exceptional: false,
        })
        .expect("the issuer gives a grant")
}

pub struct Flawed {
    inner: FakeRegistry,
    clock: ManualClock,
    /// Every coordinate that was in a `stage` or `commit` call, with its bytes.
    known: Vec<(Coordinate, Vec<u8>)>,
    flaw: Option<Flaw>,
    shape: ChannelShape,
    calls: u32,
    commit_calls: u32,
    lost: bool,
    list_lags: bool,
    group_fails: bool,
    read_offset: usize,
}

impl PublisherRig for FakePublisherRig {
    type Publisher = Flawed;

    fn publisher(&self, shape: &ChannelShape) -> Flawed {
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
        Flawed {
            inner: registry,
            clock: self.clock.clone(),
            known: Vec::new(),
            flaw: self.flaw,
            shape: *shape,
            calls: 0,
            commit_calls: 0,
            lost: false,
            list_lags: false,
            group_fails: false,
            read_offset: 0,
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
            Arrangement::RaceNextCommit(bytes) => p.inner.race_next_commit(bytes),
            Arrangement::LoseAnswerAfterCommit => p.inner.lose_answer_after_commit(next_commit),
            Arrangement::LoseAnswerBeforeCommit => p.inner.lose_answer_before_commit(next_commit),
            Arrangement::FailAfterCommit => p.inner.fail_after_commit(next_commit),
            Arrangement::CommitSetFailsWhole => {
                if p.flaw == Some(Flaw::SetCommitIsTorn) {
                    p.inner.torn_commit();
                } else {
                    p.inner.commit_set_fails_whole();
                }
            }
            Arrangement::ListLags => {
                p.list_lags = true;
                p.inner.list_lag();
            }
            Arrangement::FailGroupRead => {
                p.group_fails = true;
                p.inner.fail_group_read();
            }
            Arrangement::FailIndexRead => p.inner.fail_index_read(),
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
        let steps: Vec<ReadStep> = p
            .inner
            .read_log()
            .iter()
            .map(|read| match read {
                Read::Group => ReadStep::Group,
                Read::Index => ReadStep::Index,
            })
            .collect();
        if p.flaw != Some(Flaw::IndexIsReadBeforeTheGroup) {
            return steps;
        }
        let mut swapped = Vec::new();
        let mut at = 0;
        while at < steps.len() {
            if steps[at] == ReadStep::Group && steps.get(at + 1) == Some(&ReadStep::Index) {
                swapped.extend([ReadStep::Index, ReadStep::Group]);
                at += 2;
            } else {
                swapped.push(steps[at]);
                at += 1;
            }
        }
        swapped
    }
}

/// The first line of the secret of a grant: the scope its issuer wrote into it.
fn scope_of(g: &Grant) -> String {
    String::from_utf8_lossy(g.secret())
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// The grant `g` with its text and expiry changed by `change`.
fn rewritten(g: &Grant, change: impl FnOnce(&mut Vec<String>, &mut u64)) -> Grant {
    let text = String::from_utf8_lossy(g.secret()).into_owned();
    let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    let mut expires = g.expires_ms().unwrap_or(u64::MAX);
    change(&mut lines, &mut expires);
    Grant::issued(lines.join("\n").into_bytes(), expires)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Needs {
    Stage,
    Commit,
}

impl Flawed {
    fn is(&self, flaw: Flaw) -> bool {
        self.flaw == Some(flaw)
    }

    fn grouped(&self) -> bool {
        self.shape.group_addressed
    }

    /// A grant the registry issues itself, for a stub that lets a caller in without the grant
    /// the call needs.
    fn internal(&self, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        issue(&self.clock, &federated_repository(), phase, coordinates)
    }

    /// Whether `g` is no grant on a private channel and `flaw` is the flaw of the stub.
    fn lets_a_stranger_in(&self, flaw: Flaw, g: &Grant) -> bool {
        self.is(flaw) && self.shape.private && g.is_none()
    }

    fn remember(&mut self, items: &[StageItem<'_>]) {
        for item in items {
            self.known
                .push((item.coordinate.clone(), item.file.bytes.to_vec()));
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
    fn held_now(&mut self, k: &Coordinate) -> Option<Held> {
        let grant = self.internal(GrantPhase::Stage, std::slice::from_ref(k));
        match self.inner.observe(&grant, k, &[]) {
            Ok(Presence::Present(held) | Presence::Yanked(held) | Presence::Staged(held)) => {
                Some(held)
            }
            _ => None,
        }
    }

    /// The staged coordinates of the channel `ch` whose name starts with `prefix`, as listed.
    fn staged_listed(&mut self, ch: &str, prefix: &str) -> Vec<Listed> {
        let staged = self.known_where(|me, c| {
            me.inner.is_staged(c) && c.channel == ch && c.name.starts_with(prefix)
        });
        staged
            .into_iter()
            .filter_map(|coordinate| {
                self.held_now(&coordinate)
                    .map(|held| Listed { coordinate, held })
            })
            .collect()
    }

    /// The grant a flawed registry would take `g` for, when it takes it for another.
    fn regrant(&self, g: &Grant, needs: Needs) -> Option<Grant> {
        if g.is_none() {
            return None;
        }
        let scope = scope_of(g);
        let wanted = if needs == Needs::Stage {
            "stage"
        } else {
            "commit"
        };
        match self.flaw {
            Some(Flaw::AForeignGrantIsAccepted) => Some(rewritten(g, |lines, _| {
                LOCATION.clone_into(&mut lines[1]);
            })),
            Some(Flaw::TheWrongPhaseIsAccepted) if scope != wanted && scope != "read" => {
                Some(rewritten(g, |lines, _| wanted.clone_into(&mut lines[0])))
            }
            Some(Flaw::AnExpiredGrantIsAccepted) => {
                Some(rewritten(g, |_, expires| *expires = u64::MAX))
            }
            Some(Flaw::AReadGrantMayWrite) if scope == "read" => {
                Some(rewritten(g, |lines, _| wanted.clone_into(&mut lines[0])))
            }
            _ => None,
        }
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
        self.inner.commit_unit()
    }

    fn conditional_put(&self) -> bool {
        self.inner.conditional_put()
    }

    fn grant_limit(&self) -> usize {
        self.inner.grant_limit()
    }

    fn observe(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        set: &[StageItem<'_>],
    ) -> Result<Presence, SeamError> {
        self.calls += 1;
        let first = self.calls == 1;
        let stranger_in;
        let g = if self.lets_a_stranger_in(Flaw::StrangerObservesPrivate, g) {
            stranger_in = self.internal(GrantPhase::Read, std::slice::from_ref(k));
            &stranger_in
        } else {
            g
        };
        let staged = self.inner.is_staged(k);
        let scope = scope_of(g);
        let told: &[StageItem<'_>] = if self.is(Flaw::TheDeclaredSetIsDropped) && staged {
            &[]
        } else {
            set
        };
        let result = if self.is(Flaw::ObserveReadsTheLaggingList) && self.list_lags {
            Ok(Presence::Absent)
        } else {
            self.inner.observe(g, k, told)
        };
        let granted = !g.is_none();
        match (self.flaw, result) {
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
            (Some(Flaw::TheStampIsDropped), Ok(Presence::Present(held))) => {
                Ok(Presence::Present(Held {
                    stamp: None,
                    ..held
                }))
            }
            (Some(Flaw::GrantedObserveIsBlindToOthers), Ok(Presence::Present(held)))
                if granted && !self.grouped() && held.stamp.is_none() =>
            {
                Ok(Presence::Absent)
            }
            (Some(Flaw::GroupedObserveIsBlindToOthers), Ok(Presence::Present(held)))
                if granted && self.grouped() && !set.is_empty() && held.stamp.is_none() =>
            {
                Ok(Presence::Absent)
            }
            (Some(Flaw::FailedGroupReadIsAbsent), Err(SeamError::Unknown(_)))
                if self.group_fails =>
            {
                Ok(Presence::Absent)
            }
            (Some(Flaw::AnEmptySetSeesStagedState), Ok(Presence::Absent))
                if set.is_empty() && granted && staged && self.grouped() =>
            {
                Ok(Presence::Staged(Held::default()))
            }
            (_, result) => result,
        }
    }

    fn stage(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<StageAnswer, SeamError> {
        self.calls += 1;
        self.remember(items);
        let regranted = self.regrant(g, Needs::Stage);
        let g = regranted.as_ref().unwrap_or(g);
        let covering;
        let g = if self.is(Flaw::StageIgnoresCoverage) && scope_of(g) == "stage" {
            covering = covering_every(g, items);
            &covering
        } else {
            g
        };
        if let Some(first) = items.first() {
            let held = self.inner.bytes(first.coordinate).map(<[u8]>::to_vec);
            let ours = first.file.bytes.to_vec();
            let yanked = self.inner.is_yanked(first.coordinate);
            if self.is(Flaw::StageOverTheSameBytesIsStaged)
                && held.as_ref() == Some(&ours)
                && !yanked
            {
                return Ok(StageAnswer::Staged);
            }
            if self.is(Flaw::StageOverOtherBytesIsStaged)
                && held.as_ref().is_some_and(|theirs| *theirs != ours)
            {
                return Ok(StageAnswer::Staged);
            }
        }
        self.inner.stage(g, items)
    }

    fn abort(&mut self, g: &Grant, ks: &[Coordinate]) -> Result<(), SeamError> {
        self.calls += 1;
        if self.is(Flaw::AbortDoesNothing) && scope_of(g) == "stage" {
            return Ok(());
        }
        if self.is(Flaw::AbortGivesBackAllStaged) && scope_of(g) == "stage" {
            let mut all = self.known_where(|me, c| me.inner.is_staged(c));
            for k in ks {
                if !all.contains(k) {
                    all.push(k.clone());
                }
            }
            let grant = self.internal(GrantPhase::Stage, &all);
            return self.inner.abort(&grant, &all);
        }
        let regranted = self.regrant(g, Needs::Stage);
        self.inner.abort(regranted.as_ref().unwrap_or(g), ks)
    }

    fn commit(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<CommitAnswer, SeamError> {
        self.calls += 1;
        self.remember(items);
        let regranted = self.regrant(g, Needs::Commit);
        let g = regranted.as_ref().unwrap_or(g);
        let covering;
        let g = if self.is(Flaw::CommitIgnoresCoverage) && scope_of(g) == "commit" {
            covering = covering_every(g, items);
            &covering
        } else {
            g
        };
        self.commit_calls += 1;
        let first = items.first();
        let was_staged = first.is_some_and(|item| self.inner.is_staged(item.coordinate));
        if let Some(item) = first {
            let held = self.inner.bytes(item.coordinate).map(<[u8]>::to_vec);
            let ours = item.file.bytes.to_vec();
            if self.is(Flaw::CommitOverYankedCommits) && self.inner.is_yanked(item.coordinate) {
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
        match (self.flaw, result, first) {
            (Some(Flaw::ARacedCommitOverwrites), Ok(CommitAnswer::Duplicate), Some(item))
                if was_staged =>
            {
                self.inner
                    .present_different(item.coordinate, item.file.bytes);
                Ok(CommitAnswer::Committed)
            }
            (Some(Flaw::TransientAfterCommitIsPermanent), Err(SeamError::Transient(_)), _) => {
                Err(SeamError::Permanent("the commit failed".to_owned()))
            }
            (_, Err(SeamError::Unknown(why)), _) => {
                self.lost = true;
                Err(SeamError::Unknown(why))
            }
            (_, result, _) => result,
        }
    }

    fn yank(&mut self, g: &Grant, k: &Coordinate, reason: &str) -> Result<(), SeamError> {
        self.calls += 1;
        if self.is(Flaw::YankIgnoresThePhase) && scope_of(g) == "stage" {
            let grant = self.internal(GrantPhase::Commit, std::slice::from_ref(k));
            return self.inner.yank(&grant, k, reason);
        }
        let regranted = self.regrant(g, Needs::Commit);
        self.inner.yank(regranted.as_ref().unwrap_or(g), k, reason)
    }

    fn fetch(&mut self, g: &Grant, k: &Coordinate) -> Result<Option<Vec<u8>>, SeamError> {
        self.calls += 1;
        if self.is(Flaw::StrangerFetchesStaged) && g.is_none() && self.inner.is_staged(k) {
            let bytes = self.known.iter().rev().find(|(c, _)| c == k);
            return Ok(bytes.map(|(_, bytes)| bytes.clone()));
        }
        let stranger_in;
        let g = if self.lets_a_stranger_in(Flaw::StrangerFetchesPrivate, g) {
            stranger_in = self.internal(GrantPhase::Read, std::slice::from_ref(k));
            &stranger_in
        } else {
            g
        };
        if self.is(Flaw::AnonymousFetchIsDenied) && g.is_none() {
            return Err(SeamError::GrantRefused(
                ucdk_release::codec::GrantFault::Denied,
            ));
        }
        if self.is(Flaw::YankedIsNotFetchable) && self.inner.is_yanked(k) {
            return Ok(None);
        }
        self.inner.fetch(g, k)
    }

    fn newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError> {
        if self.is(Flaw::AnonymousFetchIsDenied) && g.is_none() {
            return Err(SeamError::GrantRefused(
                ucdk_release::codec::GrantFault::Denied,
            ));
        }
        let stranger_in;
        let g = if self.lets_a_stranger_in(Flaw::StrangerNewestPrivate, g) {
            stranger_in = self.internal(GrantPhase::Read, &[]);
            &stranger_in
        } else {
            g
        };
        let mut found = self.inner.newest(g, ch, name)?;
        if self.is(Flaw::NewestDropsYanked) {
            found.retain(|(coordinate, _)| !self.inner.is_yanked(coordinate));
        }
        if self.is(Flaw::NewestIncludesStaged) {
            for listed in self.staged_listed(ch, name) {
                if listed.coordinate.name == name {
                    found.push((listed.coordinate, listed.held));
                }
            }
        }
        if self.is(Flaw::NewestMatchesByPrefix) || self.is(Flaw::NewestIgnoresTheChannel) {
            let by_prefix = self.is(Flaw::NewestMatchesByPrefix);
            let held = self.known_where(|me, c| {
                me.inner.bytes(c).is_some()
                    && if by_prefix {
                        c.channel == ch && c.name.starts_with(name)
                    } else {
                        c.name == name
                    }
            });
            found = held
                .into_iter()
                .filter_map(|c| self.held_now(&c).map(|held| (c, held)))
                .collect();
        }
        Ok(found)
    }

    fn list(&mut self, g: &Grant, scope: &ListScope) -> Result<Vec<Listed>, SeamError> {
        if self.is(Flaw::AnonymousFetchIsDenied) && g.is_none() {
            return Err(SeamError::GrantRefused(
                ucdk_release::codec::GrantFault::Denied,
            ));
        }
        let stranger_in;
        let g = if self.lets_a_stranger_in(Flaw::StrangerListsPrivate, g) {
            stranger_in = self.internal(GrantPhase::Read, &[]);
            &stranger_in
        } else {
            g
        };
        let mut listed = self.inner.list(g, scope)?;
        if self.is(Flaw::ListIncludesStaged) {
            listed.extend(self.staged_listed(&scope.channel, &scope.prefix));
        }
        Ok(listed)
    }
}

/// The grant `g` as if it had been issued for every coordinate of `items`: the same scope,
/// repository and expiry.
fn covering_every(g: &Grant, items: &[StageItem<'_>]) -> Grant {
    rewritten(g, |lines, _| {
        lines.truncate(2);
        lines.extend(items.iter().map(|item| coordinate_key(item.coordinate)));
    })
}
