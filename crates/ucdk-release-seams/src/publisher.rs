// SPDX-License-Identifier: Apache-2.0
//! A registry: observe, stage, commit, abort and read what a channel holds.
//!
//! There is no delete and no force parameter, and the contract allows no overwrite: the write is
//! an atomic create-if-absent in both phases, and a call that finds the coordinate created
//! meanwhile answers `Duplicate` and leaves the bytes that are there.

use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::{ArtifactType, Coordinate};

use crate::{Grant, SeamError};

/// The bytes of one member, under the name the manifest gives them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blob<'a> {
    pub name: &'a str,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageAnswer {
    Staged,
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitAnswer {
    Committed,
    Duplicate,
}

/// What one commit call makes readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitUnit {
    /// Every coordinate the call names, or none.
    Set,
    /// Each coordinate on its own.
    Coordinate,
}

/// One coordinate of a Publish action, with the bytes and the `Held` it is written with, so a
/// registry can store the stamp and the exceptional mark beside the bytes.
#[derive(Debug, Clone, Copy)]
pub struct StageItem<'a> {
    pub coordinate: &'a Coordinate,
    pub file: &'a Blob<'a>,
    pub held: &'a Held,
}

/// What `list` is asked about: the coordinates of one channel whose name starts with `prefix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListScope {
    pub channel: String,
    pub prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub coordinate: Coordinate,
    pub held: Held,
}

pub trait Publisher: Send {
    fn types(&self) -> &[ArtifactType];

    /// True if the channel holds staged bytes that are not yet readable. False: `stage` and
    /// `abort` are never called and `commit` carries the bytes.
    fn stages(&self) -> bool;

    fn commit_unit(&self) -> CommitUnit;

    /// True only if a commit is an atomic create-if-absent of the coordinate.
    fn conditional_put(&self) -> bool;

    /// The most coordinates one grant may cover; `usize::MAX` when the channel has no limit.
    fn grant_limit(&self) -> usize;

    /// A point read of `k`, decided by fetching and hashing. `set` is the declared items of
    /// the action `k` belongs to, `k` among them: a registry that addresses staged state by a
    /// group derived from the whole set finds "staged with our digest" only when told the set.
    /// It is empty when the caller has no set, and then only what is readable by coordinate can
    /// be seen. A non-empty `set` that does not hold `k` is `Refused`. For a private registry
    /// the read is a group read, skipped for an empty `set`, then an index read, with the same
    /// grant; the index answer wins when it found `k`, so a coordinate committed by another
    /// writer reads as `Present` and never `Absent`, and a read that fails fails the observe.
    fn observe(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        set: &[StageItem<'_>],
    ) -> Result<Presence, SeamError>;

    /// Creates the staged state, atomically and only where the coordinate is neither held nor
    /// staged.
    fn stage(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<StageAnswer, SeamError>;

    /// Gives staged state back; never touches a committed coordinate.
    fn abort(&mut self, g: &Grant, ks: &[Coordinate]) -> Result<(), SeamError>;

    /// Makes the staged coordinates readable.
    fn commit(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<CommitAnswer, SeamError>;

    fn yank(&mut self, g: &Grant, k: &Coordinate, reason: &str) -> Result<(), SeamError>;

    fn fetch(&mut self, g: &Grant, k: &Coordinate) -> Result<Option<Vec<u8>>, SeamError>;

    /// Every coordinate of the name the channel holds, of any version and platform, yanked ones
    /// included, each with its `Held`. It compares nothing.
    fn newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError>;

    /// May lag; used for reports and never to decide an effect.
    fn list(&mut self, g: &Grant, scope: &ListScope) -> Result<Vec<Listed>, SeamError>;
}
