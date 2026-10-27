// SPDX-License-Identifier: Apache-2.0
//! The stubs of the publisher that break one promise of a registry: the conforming fake wrapped
//! so that it answers one call wrongly. The rig that builds them is in `publisher_fake`.

use ucdk_core::{Clock, ManualClock};
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::{Held, Presence};
use ucdk_release_fake::{FakeRegistry, coordinate_key};
use ucdk_release_proto::v1::{ArtifactType, Coordinate, Reason};
use ucdk_release_seams::kit::{ChannelShape, LOCATION, federated_repository};
use ucdk_release_seams::{
    CommitAnswer, CommitUnit, Grant, ListScope, Listed, Publisher, SeamError, StageAnswer,
    StageItem, YankUnit,
};

use super::publisher_access::{Needs, covering_every, covers_last};
use super::publisher_client::Client;
use super::publisher_grant::{issue, scope_of};
use super::secret::texts;

pub use super::publisher_fake::FakePublisherRig;
pub use super::publisher_flaw::{Call, Flaw};

/// A stub of the fake registry: the conforming fake with one promise broken, or with none.
pub struct Flawed {
    pub(super) inner: Client,
    pub(super) clock: ManualClock,
    /// Every coordinate that was in a `stage` or `commit` call, with its bytes.
    pub(super) known: Vec<(Coordinate, Vec<u8>)>,
    /// The digest each coordinate was declared with, when it was declared one.
    declared: Vec<(Coordinate, String)>,
    /// Every coordinate that was in a call at a place after the first.
    later: Vec<Coordinate>,
    banned: Vec<Coordinate>,
    /// Each coordinate the channel takes only in a call that names its base, with the base.
    paired: Vec<(Coordinate, Coordinate)>,
    /// A private channel refuses `fetch`, `newest` and `list` to a stage grant and a commit grant,
    /// which are for `observe` only.
    refuses_writer_reads: bool,
    pub(super) flaw: Option<Flaw>,
    pub(super) shape: ChannelShape,
    seals: bool,
    version_yank: bool,
    calls: u32,
    pub(super) commit_calls: u32,
    lost: bool,
    /// A commit was arranged to lose a race.
    pub(super) raced: bool,
    /// The staged state was arranged to vanish.
    pub(super) vanished: bool,
    /// A commit was arranged to become readable late.
    pub(super) becoming: bool,
    /// The coordinates that were arranged to be burned.
    pub(super) burned: Vec<Coordinate>,
    pub(super) list_lags: bool,
    /// The group read of an observe was arranged to fail.
    pub(super) group_fails: bool,
}

impl Flawed {
    /// A stub around `registry`: `banned` are the coordinates it cannot hold, `flaw` the promise
    /// it breaks, and `(seals, version_yank)` what it says of a version.
    pub(super) fn new(
        registry: FakeRegistry,
        clock: ManualClock,
        banned: Vec<Coordinate>,
        flaw: Option<Flaw>,
        shape: ChannelShape,
        (seals, version_yank): (bool, bool),
    ) -> Self {
        let sends = match flaw {
            Some(Flaw::SendsRefused(defect)) => Some(defect),
            _ => None,
        };
        Self {
            inner: Client::new(registry, clock.clone(), sends),
            clock,
            known: Vec::new(),
            declared: Vec::new(),
            later: Vec::new(),
            banned,
            paired: Vec::new(),
            refuses_writer_reads: false,
            flaw,
            shape,
            seals,
            version_yank,
            calls: 0,
            commit_calls: 0,
            lost: false,
            raced: false,
            vanished: false,
            becoming: false,
            burned: Vec::new(),
            list_lags: false,
            group_fails: false,
        }
    }

    /// The same channel taking each `(sibling, base)` coordinate only in a call that names its
    /// base, and refusing the stage and the commit of a call that names the sibling alone.
    #[must_use]
    pub(super) fn taking_only_with_the_base(self, paired: Vec<(Coordinate, Coordinate)>) -> Self {
        Self { paired, ..self }
    }

    /// The same channel refusing `fetch`, `newest` and `list` to a stage grant and a commit grant
    /// when it is private.
    #[must_use]
    pub(super) fn refusing_writer_reads(self) -> Self {
        Self {
            refuses_writer_reads: true,
            ..self
        }
    }

    /// How many requests the publisher has sent to the registry.
    pub(super) fn sent(&self) -> usize {
        self.inner.sent()
    }

    /// Whether a call over `ks` names a coordinate the channel takes only with its base and not the
    /// base.
    fn names_a_sibling_alone(&self, ks: &[Coordinate]) -> bool {
        self.paired
            .iter()
            .any(|(sibling, base)| ks.contains(sibling) && !ks.contains(base))
    }

    /// Whether a read with `g` is one a private channel refuses because the grant is for
    /// `observe` only.
    fn refuses_to_read_for(&self, g: &Grant) -> bool {
        self.refuses_writer_reads
            && self.shape.private
            && matches!(scope_of(g).as_str(), "stage" | "commit")
    }

    pub(super) fn is(&self, flaw: Flaw) -> bool {
        self.flaw == Some(flaw)
    }

    fn grouped(&self) -> bool {
        self.shape.group_addressed
    }

    /// A grant the registry issues itself, for a stub that lets a caller in without the grant
    /// the call needs.
    pub(super) fn internal(&self, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
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
        for (at, item) in items.iter().enumerate() {
            if at > 0 {
                self.later.push(item.coordinate.clone());
            }
            self.known
                .push((item.coordinate.clone(), item.file.bytes.to_vec()));
            if !item.held.digest.is_empty() {
                self.declared
                    .push((item.coordinate.clone(), item.held.digest.clone()));
            }
        }
    }

    /// The coordinates a call once named that satisfy `keep`, each once.
    pub(super) fn known_where(&self, keep: impl Fn(&Self, &Coordinate) -> bool) -> Vec<Coordinate> {
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

    /// The held coordinates that agree with `k` in channel, type and name, of any version.
    fn held_of_the_name(&self, k: &Coordinate) -> Vec<Coordinate> {
        self.known_where(|me, c| {
            me.inner.bytes(c).is_some()
                && c.channel == k.channel
                && c.r#type == k.r#type
                && c.name == k.name
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
        let stranger_in;
        let g = if self.lets_a_stranger_in(Flaw::StrangerObservesPrivate, g) {
            stranger_in = self.internal(GrantPhase::Read, std::slice::from_ref(k));
            &stranger_in
        } else {
            g
        };
        let with_set;
        let g = if self.is(Flaw::StrangerObservesPrivateWithASet)
            && self.shape.private
            && g.is_none()
            && !set.is_empty()
        {
            with_set = self.internal(GrantPhase::Stage, std::slice::from_ref(k));
            &with_set
        } else {
            g
        };
        let lax;
        let g = if let Some(grant) = self.lax_read(g, Some(k)) {
            lax = grant;
            &lax
        } else {
            g
        };
        let staged = self.inner.is_staged(k);
        let scope = scope_of(g);
        let told: &[StageItem<'_>] = if (self.is(Flaw::TheDeclaredSetIsDropped) && staged)
            || (self.is(Flaw::ObserveDropsAForeignSet)
                && !set.iter().any(|item| item.coordinate == k))
        {
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
            (
                Some(
                    Flaw::ObserveIgnoresVersion
                    | Flaw::ObserveIgnoresPlatform
                    | Flaw::ObserveIgnoresChannel,
                ),
                Ok(Presence::Absent),
            ) => match self.sibling(k) {
                Some(sibling) => {
                    let read = self.internal(GrantPhase::Read, std::slice::from_ref(&sibling));
                    self.inner.observe(&read, &sibling, &[])
                }
                None => Ok(Presence::Absent),
            },
            (Some(Flaw::AbsentIsAnError), Ok(Presence::Absent))
                if first && (!granted || (self.shape.private && scope == "read")) =>
            {
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
            (Some(Flaw::TheMarkIsDropped), Ok(Presence::Present(held))) => {
                Ok(Presence::Present(Held {
                    exceptional: false,
                    ..held
                }))
            }
            (Some(Flaw::LaterMembersHaveAWrongDigest), Ok(Presence::Present(held)))
                if self.later.contains(k) =>
            {
                Ok(Presence::Present(digest_of(held, "bad")))
            }
            (Some(Flaw::LaterMembersLoseTheirStamp), Ok(Presence::Present(held)))
                if self.later.contains(k) =>
            {
                Ok(Presence::Present(Held {
                    stamp: None,
                    ..held
                }))
            }
            (Some(Flaw::LaterMembersLoseTheirMark), Ok(Presence::Present(held)))
                if self.later.contains(k) =>
            {
                Ok(Presence::Present(Held {
                    exceptional: false,
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
        if self.names_a_sibling_alone(&coordinates) {
            return Err(SeamError::Refused(Reason::Channel));
        }
        let staged_before = items
            .first()
            .and_then(|item| self.staged_bytes(item.coordinate));
        self.remember(items);
        let stranger_in =
            self.writer_for_a_stranger(Flaw::StrangerMayStage, g, GrantPhase::Stage, &coordinates);
        let g = stranger_in.as_ref().unwrap_or(g);
        let regranted = self.regrant(g, Needs::Stage);
        let g = regranted.as_ref().unwrap_or(g);
        let lax = self.lax_write(g, Call::Stage, &coordinates);
        let g = lax.as_ref().unwrap_or(g);
        let covering;
        let g = if (self.is(Flaw::StageIgnoresCoverage) && scope_of(g) == "stage")
            || (self.is(Flaw::OnlyTheLastCoordinateIsCovered) && covers_last(g, items))
        {
            covering = covering_every(g, items);
            &covering
        } else {
            g
        };
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
        let stranger_in =
            self.writer_for_a_stranger(Flaw::StrangerMayAbort, g, GrantPhase::Stage, ks);
        let g = stranger_in.as_ref().unwrap_or(g);
        let regranted = self.regrant(g, Needs::Stage);
        let g = regranted.as_ref().unwrap_or(g);
        let lax = self.lax_write(g, Call::Abort, ks);
        self.inner.abort(lax.as_ref().unwrap_or(g), ks)
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
        if self.names_a_sibling_alone(&coordinates) {
            return Err(SeamError::Refused(Reason::Channel));
        }
        self.remember(items);
        let stranger_in = self.writer_for_a_stranger(
            Flaw::StrangerMayCommit,
            g,
            GrantPhase::Commit,
            &coordinates,
        );
        let g = stranger_in.as_ref().unwrap_or(g);
        let regranted = self.regrant(g, Needs::Commit);
        let g = regranted.as_ref().unwrap_or(g);
        let lax = self.lax_write(g, Call::Commit, &coordinates);
        let g = lax.as_ref().unwrap_or(g);
        let covering;
        let g = if (self.is(Flaw::CommitIgnoresCoverage) && scope_of(g) == "commit")
            || (self.is(Flaw::OnlyTheLastCoordinateIsCovered) && covers_last(g, items))
        {
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
            (Some(Flaw::ARacedCommitOverwrites), Ok(CommitAnswer::Duplicate))
                if was_staged && self.raced =>
            {
                if let Some(item) = first {
                    self.inner
                        .present_different(item.coordinate, item.file.bytes);
                }
                Ok(CommitAnswer::Committed)
            }
            (Some(Flaw::TransientAfterCommitIsPermanent), Err(SeamError::Transient(_)))
                if self.shape.stages =>
            {
                Err(SeamError::Permanent("the commit failed".to_owned()))
            }
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
        if self.is(Flaw::YankIgnoresThePhase) && scope_of(g) == "stage" {
            let grant = self.internal(GrantPhase::Commit, std::slice::from_ref(k));
            return self.inner.yank(&grant, k, reason);
        }
        let stranger_in = self.writer_for_a_stranger(
            Flaw::StrangerMayYank,
            g,
            GrantPhase::Commit,
            std::slice::from_ref(k),
        );
        let g = stranger_in.as_ref().unwrap_or(g);
        let regranted = self.regrant(g, Needs::Commit);
        let g = regranted.as_ref().unwrap_or(g);
        let lax = self.lax_write(g, Call::Yank, std::slice::from_ref(k));
        let g = lax.as_ref().unwrap_or(g);
        if self.version_yank && self.inner.bytes(k).is_some() {
            let held = self.held_of_the_version(k);
            if self.is(Flaw::VersionYankIsPerCoordinate) {
                return self.inner.yank(g, k, reason);
            }
            // A grant that is bad for the coordinate itself is refused as the registry refuses
            // it, for the reason it gives; one that is good for it and leaves a sibling out is
            // `Denied`.
            if !self.is_good_for(g, "commit", std::slice::from_ref(k)) {
                return self.inner.yank(g, k, reason);
            }
            if !self.is(Flaw::VersionYankIgnoresTheGrant) && !self.is_good_for(g, "commit", &held) {
                return Err(SeamError::GrantRefused(GrantFault::Denied));
            }
            let yanked = if self.is(Flaw::VersionYankReachesEveryVersion) {
                self.held_of_the_name(k)
            } else {
                held
            };
            for sibling in yanked {
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
            return Err(SeamError::GrantRefused(GrantFault::Denied));
        }
        if self.is(Flaw::YankedIsNotFetchable) && self.inner.is_yanked(k) {
            return Ok(None);
        }
        let lax;
        let g = if let Some(grant) = self.lax_read(g, Some(k)) {
            lax = grant;
            &lax
        } else {
            g
        };
        let found = self.inner.fetch(g, k)?;
        if self.refuses_to_read_for(g) {
            return Err(SeamError::GrantRefused(GrantFault::Denied));
        }
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
            let read = self.internal(GrantPhase::Read, std::slice::from_ref(&sibling));
            return self.inner.fetch(&read, &sibling);
        }
        Ok(found)
    }

    fn newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError> {
        if self.is(Flaw::AnonymousFetchIsDenied) && g.is_none() {
            return Err(SeamError::GrantRefused(GrantFault::Denied));
        }
        let stranger_in;
        let g = if self.lets_a_stranger_in(Flaw::StrangerNewestPrivate, g) {
            stranger_in = self.internal(GrantPhase::Read, &[]);
            &stranger_in
        } else {
            g
        };
        let lax;
        let g = if let Some(grant) = self.lax_read(g, None) {
            lax = grant;
            &lax
        } else {
            g
        };
        let mut found = self.inner.newest(g, ch, name)?;
        if self.refuses_to_read_for(g) {
            return Err(SeamError::GrantRefused(GrantFault::Denied));
        }
        if self.is(Flaw::NewestIsEmpty) {
            found.clear();
        }
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
        for (_, held) in &mut found {
            *held = self.misreported(held.clone(), Flaw::NewestWrongDigest);
            if self.is(Flaw::NewestDropsTheStamp) {
                held.stamp = None;
            }
            if self.is(Flaw::NewestDropsTheExceptionalMark) {
                held.exceptional = false;
            }
        }
        Ok(found)
    }

    fn list(&mut self, g: &Grant, scope: &ListScope) -> Result<Vec<Listed>, SeamError> {
        if self.is(Flaw::AnonymousFetchIsDenied) && g.is_none() {
            return Err(SeamError::GrantRefused(GrantFault::Denied));
        }
        let stranger_in;
        let g = if self.lets_a_stranger_in(Flaw::StrangerListsPrivate, g) {
            stranger_in = self.internal(GrantPhase::Read, &[]);
            &stranger_in
        } else {
            g
        };
        let lax;
        let g = if let Some(grant) = self.lax_read(g, None) {
            lax = grant;
            &lax
        } else {
            g
        };
        let mut listed = self.inner.list(g, scope)?;
        if self.refuses_to_read_for(g) {
            return Err(SeamError::GrantRefused(GrantFault::Denied));
        }
        if self.is(Flaw::ListIsEmpty) {
            listed.clear();
        }
        if self.is(Flaw::ListIncludesStaged) {
            listed.extend(self.staged_listed(&scope.channel, &scope.prefix));
        }
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
