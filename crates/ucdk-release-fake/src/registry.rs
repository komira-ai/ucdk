// SPDX-License-Identifier: Apache-2.0
//! A registry in memory: coordinate to bytes plus a yank mark, with staged state beside the
//! committed one. It checks every grant against the shared clock, its own scopes and the
//! coordinates the grant covers, and it logs what it was asked, in order.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest as _, Sha256};
use ucdk_core::{Clock, Hash256, ManualClock};
use ucdk_release::codec::GrantFault;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::{ArtifactType, Coordinate, Reason};
use ucdk_release_seams::{
    Backoff, CommitAnswer, CommitUnit, Grant, ListScope, Listed, Publisher, SeamError, StageAnswer,
    StageItem, YankUnit,
};

use crate::token::{coordinate_key, decode};

/// The scope names the registry accepts for each kind of call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scopes {
    pub stage: String,
    pub commit: String,
    pub read: String,
}

impl Default for Scopes {
    fn default() -> Self {
        Self {
            stage: "stage".to_owned(),
            commit: "commit".to_owned(),
            read: "read".to_owned(),
        }
    }
}

/// One read an observe made, for the registries that address staged state by group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Read {
    Group,
    Index,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Need {
    Stage,
    Commit,
    Read,
}

struct Stored {
    coordinate: Coordinate,
    bytes: Vec<u8>,
    held: Held,
    yanked: bool,
}

struct Staged {
    stored: Stored,
    group: String,
}

#[derive(Default)]
struct Faults {
    lose_answer_after_commit: Option<u32>,
    lose_answer_before_commit: Option<u32>,
    fail_after_commit: Option<u32>,
    read_lag: u32,
    list_lag: bool,
    race_next_commit: Option<Vec<u8>>,
    torn_commit: bool,
    rate_limit: u32,
    no_digest_exposed: bool,
    unreadable_after: Option<u32>,
    lose_staged_state: bool,
    others_staged_visible: bool,
    commit_set_fails_whole: bool,
    fail_group_read: bool,
    fail_index_read: bool,
    grants_used: bool,
}

pub struct FakeRegistry {
    clock: ManualClock,
    repository: String,
    scopes: Scopes,
    types: Vec<ArtifactType>,
    staging: bool,
    unit: CommitUnit,
    conditional: bool,
    group_addressed: bool,
    open: bool,
    private: bool,
    limit: usize,
    faults: Faults,
    committed: BTreeMap<String, Stored>,
    staged: BTreeMap<String, Staged>,
    others_staged: BTreeMap<String, Stored>,
    burned: BTreeSet<String>,
    commit_calls: u32,
    observes: u32,
    commits: Vec<Vec<String>>,
    stages: Vec<Vec<String>>,
    calls: Vec<String>,
    reads: Vec<Read>,
}

/// The identity digest of `bytes`: what a registry reports after fetching and hashing.
fn digest(bytes: &[u8]) -> String {
    Hash256::from_sha256(&Sha256::digest(bytes).into()).to_string()
}

fn keys(items: &[StageItem<'_>]) -> Vec<String> {
    items.iter().map(|i| coordinate_key(i.coordinate)).collect()
}

/// Refuses a set that names one coordinate twice: its two copies could carry different bytes,
/// and a registry cannot hold both.
fn distinct(ks: &[String]) -> Result<(), SeamError> {
    let mut seen = BTreeSet::new();
    if ks.iter().any(|k| !seen.insert(k)) {
        return Err(SeamError::Permanent(
            "the set names a coordinate twice".to_owned(),
        ));
    }
    Ok(())
}

/// The group a set of items is staged under: every member with its digest, so the group read of
/// an observe finds it only when told the same set.
fn group_of(items: &[StageItem<'_>]) -> String {
    let mut parts: Vec<String> = items
        .iter()
        .map(|i| {
            format!(
                "{}={}:{}",
                coordinate_key(i.coordinate),
                digest(i.file.bytes),
                i.file.bytes.len()
            )
        })
        .collect();
    parts.sort();
    parts.join("|")
}

impl FakeRegistry {
    /// A registry at `repository` that stages, commits a set at once and is conditional, and
    /// reads time from `clock`.
    pub fn new(clock: &ManualClock, repository: &str) -> Self {
        Self {
            clock: clock.clone(),
            repository: repository.to_owned(),
            scopes: Scopes::default(),
            types: vec![ArtifactType::File],
            staging: true,
            unit: CommitUnit::Set,
            conditional: true,
            group_addressed: false,
            open: false,
            private: false,
            limit: usize::MAX,
            faults: Faults::default(),
            committed: BTreeMap::new(),
            staged: BTreeMap::new(),
            others_staged: BTreeMap::new(),
            burned: BTreeSet::new(),
            commit_calls: 0,
            observes: 0,
            commits: Vec::new(),
            stages: Vec::new(),
            calls: Vec::new(),
            reads: Vec::new(),
        }
    }

    /// A channel that does not stage: `commit` carries the bytes.
    #[must_use]
    pub fn without_staging(mut self) -> Self {
        self.staging = false;
        self
    }

    /// A channel that commits each coordinate on its own: a `commit` that names more than one
    /// coordinate is refused (`Permanent`, nothing committed), so an engine that sends a set to
    /// such a channel is caught.
    #[must_use]
    pub fn commit_per_coordinate(mut self) -> Self {
        self.unit = CommitUnit::Coordinate;
        self
    }

    /// A registry whose commit may replace what is there, which declares itself unfit as a home.
    #[must_use]
    pub fn tag_replacing(mut self) -> Self {
        self.conditional = false;
        self
    }

    /// A registry that addresses staged state by the group of the declared set, and logs its
    /// group and index reads.
    #[must_use]
    pub fn group_addressed(mut self) -> Self {
        self.group_addressed = true;
        self
    }

    /// A channel that asks for no grant: writes with no grant are admitted.
    #[must_use]
    pub fn open(mut self) -> Self {
        self.open = true;
        self
    }

    /// A private repository: every read (observe, fetch, newest, list) needs a grant, as a
    /// write does. A registry is public until this is called.
    #[must_use]
    pub fn private(mut self) -> Self {
        self.private = true;
        self
    }

    #[must_use]
    pub fn with_scopes(mut self, scopes: Scopes) -> Self {
        self.scopes = scopes;
        self
    }

    #[must_use]
    pub fn with_grant_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    // ---- faults ----

    /// The `n`th commit call commits, then answers `Unknown`.
    pub fn lose_answer_after_commit(&mut self, n: u32) {
        self.faults.lose_answer_after_commit = Some(n);
    }

    /// The `n`th commit call answers `Unknown` and commits nothing.
    pub fn lose_answer_before_commit(&mut self, n: u32) {
        self.faults.lose_answer_before_commit = Some(n);
    }

    /// The `n`th commit call commits, then answers `Transient`.
    pub fn fail_after_commit(&mut self, n: u32) {
        self.faults.fail_after_commit = Some(n);
    }

    /// The next `n` index reads that would find a committed coordinate find nothing.
    pub fn read_lag(&mut self, n: u32) {
        self.faults.read_lag = n;
    }

    /// `list` never shows what was committed.
    pub fn list_lag(&mut self) {
        self.faults.list_lag = true;
    }

    /// `coordinate` is held, committed by another writer, with other bytes.
    pub fn present_different(&mut self, coordinate: &Coordinate, bytes: &[u8]) {
        self.committed.insert(
            coordinate_key(coordinate),
            Stored {
                coordinate: coordinate.clone(),
                bytes: bytes.to_vec(),
                held: Held::default(),
                yanked: false,
            },
        );
    }

    /// `coordinate` is burned: a failed group claimed its number and it can never be published.
    /// An observe answers `Burned`, a `stage` or a `commit` over it answers `Duplicate`, `fetch`
    /// finds nothing and `newest` and `list` do not return it.
    pub fn burn(&mut self, coordinate: &Coordinate) {
        self.burned.insert(coordinate_key(coordinate));
    }

    /// Another writer commits the first coordinate of the next commit call with `bytes`, just
    /// before it lands.
    pub fn race_next_commit(&mut self, bytes: &[u8]) {
        self.faults.race_next_commit = Some(bytes.to_vec());
    }

    /// A commit of several coordinates makes the first readable and answers `Unknown`.
    pub fn torn_commit(&mut self) {
        self.faults.torn_commit = true;
    }

    /// The next `n` calls of any kind are refused, rate limited, before they run.
    pub fn rate_limit(&mut self, n: u32) {
        self.faults.rate_limit = n;
    }

    /// An observe reports a held coordinate with no digest.
    pub fn no_digest_exposed(&mut self) {
        self.faults.no_digest_exposed = true;
    }

    /// After `n` observes, every observe answers `Unknown`.
    pub fn unreadable_after(&mut self, n: u32) {
        self.faults.unreadable_after = Some(n);
    }

    /// Staged coordinates vanish between `stage` and `commit`: nothing is kept at `stage`, so
    /// every reader, the stager's own grant included, finds `Absent`.
    pub fn lose_staged_state(&mut self) {
        self.faults.lose_staged_state = true;
    }

    /// `coordinate` is staged by another writer, with `bytes`. A stage over it answers
    /// `Duplicate`; an observe finds `Absent`, the staged state of another writer being held
    /// where the grant cannot see it, unless `others_staged_visible` is called.
    pub fn stage_other_bytes(&mut self, coordinate: &Coordinate, bytes: &[u8]) {
        self.others_staged.insert(
            coordinate_key(coordinate),
            Stored {
                coordinate: coordinate.clone(),
                bytes: bytes.to_vec(),
                held: Held::default(),
                yanked: false,
            },
        );
    }

    /// An observe that carries a grant sees the staged state of another writer, with that
    /// writer's digest (and, on a registry that addresses staged state by group, only when it
    /// is told a set). A read grant sees another writer's staged state but not its own, as the
    /// design intends: only the stage or commit grant of the action that staged sees its own.
    pub fn others_staged_visible(&mut self) {
        self.faults.others_staged_visible = true;
    }

    /// A commit of a set fails whole: nothing becomes readable.
    pub fn commit_set_fails_whole(&mut self) {
        self.faults.commit_set_fails_whole = true;
    }

    /// The group read of an observe fails.
    pub fn fail_group_read(&mut self) {
        self.faults.fail_group_read = true;
    }

    /// The index read of an observe fails.
    pub fn fail_index_read(&mut self) {
        self.faults.fail_index_read = true;
    }

    /// Every grant presented is one already spent on its identity token.
    pub fn grants_already_used(&mut self) {
        self.faults.grants_used = true;
    }

    // ---- what a test looks at ----

    /// The coordinates of each commit call, in order.
    pub fn commit_log(&self) -> &[Vec<String>] {
        &self.commits
    }

    /// The coordinates of each stage call that was admitted, in order.
    pub fn stage_log(&self) -> &[Vec<String>] {
        &self.stages
    }

    /// Every call, by verb and coordinate, in order, refused ones included.
    pub fn call_log(&self) -> &[String] {
        &self.calls
    }

    /// The group and index reads of the observes, in order.
    pub fn read_log(&self) -> &[Read] {
        &self.reads
    }

    /// The bytes held for `coordinate`, committed.
    pub fn bytes(&self, coordinate: &Coordinate) -> Option<&[u8]> {
        self.committed
            .get(&coordinate_key(coordinate))
            .map(|s| s.bytes.as_slice())
    }

    pub fn is_staged(&self, coordinate: &Coordinate) -> bool {
        self.staged.contains_key(&coordinate_key(coordinate))
    }

    pub fn is_yanked(&self, coordinate: &Coordinate) -> bool {
        self.committed
            .get(&coordinate_key(coordinate))
            .is_some_and(|s| s.yanked)
    }

    // ---- the checks every call makes ----

    fn gate(&mut self) -> Result<(), SeamError> {
        if self.faults.rate_limit > 0 {
            self.faults.rate_limit -= 1;
            return Err(SeamError::RateLimited(Backoff { ticks: 1 }));
        }
        Ok(())
    }

    /// Admits a call that needs `need` and covers `coords`: a read of a public repository needs
    /// no grant, and a grant of any scope may read; a write, and a read of a private repository,
    /// need the scope of their phase and every coordinate.
    fn admit(&self, g: &Grant, need: Need, coords: &[String]) -> Result<(), SeamError> {
        if g.is_none() {
            return if (need == Need::Read && !self.private) || self.open {
                Ok(())
            } else {
                Err(SeamError::GrantRefused(GrantFault::Denied))
            };
        }
        if self.faults.grants_used {
            return Err(SeamError::GrantRefused(GrantFault::Used));
        }
        if g.expires_ms().is_some_and(|e| self.clock.now_millis() >= e) {
            return Err(SeamError::GrantRefused(GrantFault::Expired));
        }
        let claim = decode(g.secret());
        let scope = claim.scope.as_str();
        let scope_ok = match need {
            Need::Stage => scope == self.scopes.stage,
            Need::Commit => scope == self.scopes.commit,
            Need::Read => {
                scope == self.scopes.stage
                    || scope == self.scopes.commit
                    || scope == self.scopes.read
            }
        };
        if claim.repository == self.repository
            && scope_ok
            && coords.iter().all(|c| claim.coordinates.contains(c))
        {
            Ok(())
        } else {
            Err(SeamError::GrantRefused(GrantFault::Denied))
        }
    }

    /// Whether `g` is a grant of an action that writes the staged state (the stage or the commit
    /// scope). A read grant made no staged state and does not see it.
    fn is_writer_grant(&self, g: &Grant) -> bool {
        if g.is_none() {
            return false;
        }
        let scope = decode(g.secret()).scope;
        scope == self.scopes.stage || scope == self.scopes.commit
    }

    fn held_of(&self, stored: &Stored) -> Held {
        Held {
            digest: if self.faults.no_digest_exposed {
                String::new()
            } else {
                digest(&stored.bytes)
            },
            ..stored.held.clone()
        }
    }
}

impl Publisher for FakeRegistry {
    fn types(&self) -> &[ArtifactType] {
        &self.types
    }

    fn stages(&self) -> bool {
        self.staging
    }

    fn commit_unit(&self) -> CommitUnit {
        self.unit
    }

    fn conditional_put(&self) -> bool {
        self.conditional
    }

    fn grant_limit(&self) -> usize {
        self.limit
    }

    fn yank_unit(&self) -> YankUnit {
        YankUnit::Coordinate
    }

    fn observe(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        set: &[StageItem<'_>],
    ) -> Result<Presence, SeamError> {
        let key = coordinate_key(k);
        self.calls.push(format!("observe {key}"));
        self.gate()?;
        self.admit(g, Need::Read, std::slice::from_ref(&key))?;
        if !set.is_empty() {
            let declared = keys(set);
            distinct(&declared)?;
            if !declared.contains(&key) {
                return Err(SeamError::Refused(Reason::Channel));
            }
        }
        self.observes += 1;
        if self
            .faults
            .unreadable_after
            .is_some_and(|n| self.observes > n)
        {
            return Err(SeamError::Unknown("the registry cannot be read".to_owned()));
        }
        let by_group = self.group_addressed && !set.is_empty();
        if by_group {
            self.reads.push(Read::Group);
            if self.faults.fail_group_read {
                return Err(SeamError::Unknown("the group read failed".to_owned()));
            }
        }
        self.reads.push(Read::Index);
        if self.faults.fail_index_read {
            return Err(SeamError::Unknown("the index read failed".to_owned()));
        }
        if self.burned.contains(&key) {
            return Ok(Presence::Burned);
        }
        if self.committed.contains_key(&key) && self.faults.read_lag > 0 {
            self.faults.read_lag -= 1;
            return Ok(Presence::Absent);
        }
        if let Some(stored) = self.committed.get(&key) {
            let held = self.held_of(stored);
            return Ok(if stored.yanked {
                Presence::Yanked(held)
            } else {
                Presence::Present(held)
            });
        }
        // Staged state is visible only to a stage or commit grant (a read grant made none), and,
        // when it is addressed by group, only to an observe that is told the set it was staged
        // under.
        let group_ok = |s: &Staged| !self.group_addressed || (by_group && s.group == group_of(set));
        let other = self.others_staged.get(&key).filter(|_| {
            self.faults.others_staged_visible && !g.is_none() && (!self.group_addressed || by_group)
        });
        Ok(match (self.staged.get(&key), other) {
            (Some(s), _) if self.is_writer_grant(g) && group_ok(s) => {
                Presence::Staged(self.held_of(&s.stored))
            }
            (None, Some(theirs)) => Presence::Staged(self.held_of(theirs)),
            _ => Presence::Absent,
        })
    }

    fn stage(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<StageAnswer, SeamError> {
        let ks = keys(items);
        self.calls.push(format!("stage {}", ks.join(",")));
        self.gate()?;
        if !self.staging {
            return Err(SeamError::Permanent(
                "the channel does not stage".to_owned(),
            ));
        }
        self.admit(g, Need::Stage, &ks)?;
        distinct(&ks)?;
        self.stages.push(ks.clone());
        let taken = |k: &String| {
            self.committed.contains_key(k)
                || self.burned.contains(k)
                || self.staged.contains_key(k)
                || self.others_staged.contains_key(k)
        };
        if ks.iter().any(taken) {
            return Ok(StageAnswer::Duplicate);
        }
        if !self.faults.lose_staged_state {
            let group = group_of(items);
            for (key, item) in ks.into_iter().zip(items) {
                let stored = Stored {
                    coordinate: item.coordinate.clone(),
                    bytes: item.file.bytes.to_vec(),
                    held: item.held.clone(),
                    yanked: false,
                };
                self.staged.insert(
                    key,
                    Staged {
                        stored,
                        group: group.clone(),
                    },
                );
            }
        }
        Ok(StageAnswer::Staged)
    }

    fn abort(&mut self, g: &Grant, ks: &[Coordinate]) -> Result<(), SeamError> {
        let keys: Vec<String> = ks.iter().map(coordinate_key).collect();
        self.calls.push(format!("abort {}", keys.join(",")));
        self.gate()?;
        self.admit(g, Need::Stage, &keys)?;
        for key in &keys {
            self.staged.remove(key);
        }
        Ok(())
    }

    fn commit(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<CommitAnswer, SeamError> {
        let ks = keys(items);
        self.calls.push(format!("commit {}", ks.join(",")));
        self.gate()?;
        self.commit_calls += 1;
        let n = self.commit_calls;
        self.admit(g, Need::Commit, &ks)?;
        distinct(&ks)?;
        if self.unit == CommitUnit::Coordinate && ks.len() > 1 {
            return Err(SeamError::Permanent(
                "the channel commits one coordinate at a time".to_owned(),
            ));
        }
        self.commits.push(ks.clone());
        if self.faults.lose_answer_before_commit == Some(n) {
            return Err(SeamError::Unknown("the answer was lost".to_owned()));
        }
        if let (Some(bytes), Some(first)) = (self.faults.race_next_commit.take(), items.first()) {
            self.present_different(first.coordinate, &bytes);
        }
        if self.faults.commit_set_fails_whole {
            return Err(SeamError::Permanent("the set commit failed".to_owned()));
        }
        if ks
            .iter()
            .any(|k| self.committed.contains_key(k) || self.burned.contains(k))
        {
            return Ok(CommitAnswer::Duplicate);
        }
        if self.staging {
            for (k, item) in ks.iter().zip(items) {
                let Some(staged) = self.staged.get(k) else {
                    return Err(SeamError::Permanent(
                        "a coordinate is not staged".to_owned(),
                    ));
                };
                if staged.stored.bytes != item.file.bytes || staged.stored.held != *item.held {
                    return Err(SeamError::Permanent(
                        "a coordinate differs from what was staged".to_owned(),
                    ));
                }
            }
        }
        let landing = if self.faults.torn_commit {
            1
        } else {
            items.len()
        };
        for (key, item) in ks.into_iter().zip(items).take(landing) {
            let stored = match self.staged.remove(&key) {
                Some(staged) => staged.stored,
                None => Stored {
                    coordinate: item.coordinate.clone(),
                    bytes: item.file.bytes.to_vec(),
                    held: item.held.clone(),
                    yanked: false,
                },
            };
            self.committed.insert(key, stored);
        }
        if self.faults.torn_commit {
            return Err(SeamError::Unknown("the commit was torn".to_owned()));
        }
        if self.faults.lose_answer_after_commit == Some(n) {
            return Err(SeamError::Unknown("the answer was lost".to_owned()));
        }
        if self.faults.fail_after_commit == Some(n) {
            return Err(SeamError::Transient(
                "the registry failed after it committed".to_owned(),
            ));
        }
        Ok(CommitAnswer::Committed)
    }

    fn yank(&mut self, g: &Grant, k: &Coordinate, _reason: &str) -> Result<(), SeamError> {
        let key = coordinate_key(k);
        self.calls.push(format!("yank {key}"));
        self.gate()?;
        self.admit(g, Need::Commit, std::slice::from_ref(&key))?;
        match self.committed.get_mut(&key) {
            Some(stored) => {
                stored.yanked = true;
                Ok(())
            }
            None => Err(SeamError::Permanent(format!("{key} is not held"))),
        }
    }

    fn fetch(&mut self, g: &Grant, k: &Coordinate) -> Result<Option<Vec<u8>>, SeamError> {
        let key = coordinate_key(k);
        self.calls.push(format!("fetch {key}"));
        self.gate()?;
        self.admit(g, Need::Read, std::slice::from_ref(&key))?;
        Ok(self.committed.get(&key).map(|s| s.bytes.clone()))
    }

    fn newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError> {
        self.calls.push(format!("newest {ch:?} {name:?}"));
        self.gate()?;
        self.admit(g, Need::Read, &[])?;
        Ok(self
            .committed
            .values()
            .filter(|s| s.coordinate.channel == ch && s.coordinate.name == name)
            .map(|s| (s.coordinate.clone(), self.held_of(s)))
            .collect())
    }

    fn list(&mut self, g: &Grant, scope: &ListScope) -> Result<Vec<Listed>, SeamError> {
        self.calls.push(format!("list {}", scope.channel));
        self.gate()?;
        self.admit(g, Need::Read, &[])?;
        if self.faults.list_lag {
            return Ok(Vec::new());
        }
        Ok(self
            .committed
            .values()
            .filter(|s| {
                s.coordinate.channel == scope.channel
                    && s.coordinate.name.starts_with(&scope.prefix)
            })
            .map(|s| Listed {
                coordinate: s.coordinate.clone(),
                held: self.held_of(s),
            })
            .collect())
    }
}
