// SPDX-License-Identifier: Apache-2.0
//! The records, the document they live in, and the compare-and-swap trait over it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ucdk_provider::{Kind, NodePath, OwnerLabel, PhysicalId, Produced};

/// Who owns a node, as a path: every segment of the node's path above its role. For `app/run`
/// that is `app`; for a node nested one level down, `app/db/run`, it is `app/db`. The first
/// segment is the top-level owner the stamp carries. Recording the whole path, not one flat
/// id, is what lets nested owners be added without changing what a record means.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OwnerPath(Vec<OwnerLabel>);

impl OwnerPath {
    /// The owner path of the node at `path`. A path of one segment is its own owner.
    pub fn of(path: &NodePath) -> Self {
        let segments: Vec<&str> = path.as_str().split('/').collect();
        let keep = segments.len().saturating_sub(1).max(1);
        Self(
            segments
                .into_iter()
                .take(keep)
                .map(|s| OwnerLabel::new(s).expect("a node path is made of valid segments"))
                .collect(),
        )
    }

    /// Every segment, outermost first. Never empty.
    pub fn segments(&self) -> &[OwnerLabel] {
        &self.0
    }

    /// The top-level owner.
    pub fn top(&self) -> &OwnerLabel {
        &self.0[0]
    }
}

impl fmt::Display for OwnerPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<&str> = self.0.iter().map(OwnerLabel::as_str).collect();
        f.write_str(&parts.join("/"))
    }
}

/// How the object came to be a record's object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// This tool created it.
    Created,
    /// Reserved. A tool that took over an object it did not create would record that here, and
    /// such an object is never deleted by a run. Nothing writes it yet.
    Adopted,
}

/// Where a record stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Written before a create or update: the call may or may not have landed.
    Intent,
    /// The object is there and matches what was last applied.
    Confirmed,
    /// Written before a delete: the object may or may not be gone.
    Reaping,
}

/// What the ledger knows about one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub path: NodePath,
    pub kind: Kind,
    /// The physical name the object carries (without any stamp encoding), so an intent whose
    /// create never returned can be found again by name.
    pub name: String,
    pub owner: OwnerPath,
    pub origin: Origin,
    pub state: State,
    /// The object's physical id; `None` until a create has returned or been found.
    pub physical: Option<PhysicalId>,
    /// The nodes this one needed: the edges deletion follows, in reverse.
    pub deps: BTreeSet<NodePath>,
    /// The non-secret outputs the object had when it was last confirmed, by output name
    /// (`URL`, `HOST`). A node that reads one of them is planned against this value. A secret
    /// output is never here.
    pub outputs: BTreeMap<String, String>,
}

impl Record {
    /// An intent to create the object of the node at `path`.
    pub fn intent(
        path: NodePath,
        kind: Kind,
        name: impl Into<String>,
        deps: BTreeSet<NodePath>,
    ) -> Self {
        Self {
            owner: OwnerPath::of(&path),
            path,
            kind,
            name: name.into(),
            origin: Origin::Created,
            state: State::Intent,
            physical: None,
            deps,
            outputs: BTreeMap::new(),
        }
    }

    /// The record holding the non-secret outputs of `produced`, in place of any held before.
    /// A secret output is dropped: the ledger never holds one.
    #[must_use]
    pub fn with_outputs(self, produced: &[Produced]) -> Self {
        let outputs = produced
            .iter()
            .filter(|p| !p.secret)
            .map(|p| (p.name.clone(), p.value.clone()))
            .collect();
        Self { outputs, ..self }
    }

    /// The record, confirmed with the object's physical id.
    #[must_use]
    pub fn confirmed(self, physical: PhysicalId) -> Self {
        Self {
            state: State::Confirmed,
            physical: Some(physical),
            ..self
        }
    }

    /// The record in another state, everything else kept.
    #[must_use]
    pub fn in_state(self, state: State) -> Self {
        Self { state, ..self }
    }
}

/// The version of the document. A write names the token it read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token(pub(crate) u64);

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// The whole document at one version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub token: Token,
    pub records: BTreeMap<NodePath, Record>,
}

/// One change to the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Writes the record for its path, replacing any earlier one.
    Put(Record),
    /// Drops the record of a path; a path with none is left as it is.
    Remove(NodePath),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    /// The document changed since the token was read: nothing was written.
    Stale,
    /// The ledger could not be reached: nothing was written.
    Unavailable(String),
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale => f.write_str("the ledger changed since it was read"),
            Self::Unavailable(why) => write!(f, "the ledger is unavailable: {why}"),
        }
    }
}

impl std::error::Error for LedgerError {}

/// A ledger: one document with compare-and-swap writes. Methods take `&self` so that two
/// engines can hold one ledger; an implementation does its own locking.
pub trait Ledger: Send + Sync {
    /// The document as it is now.
    fn read(&self) -> Result<Snapshot, LedgerError>;

    /// Applies `change` if the document is still at `expected`, and returns the new token.
    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError>;
}
