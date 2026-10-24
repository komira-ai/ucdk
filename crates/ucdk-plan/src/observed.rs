// SPDX-License-Identifier: Apache-2.0
//! The state a plan is compared with, as plain data: what the cloud shows and what the ledger
//! records, per node. Reading either is somebody else's job (the engine, against a provider
//! and a ledger); nothing here calls one.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_provider::{Deletability, Kind, Live, NodePath, PhysicalId, StateClass};

/// The ledger's version token. A plan is compared against one snapshot, so the token names
/// that snapshot; it is not part of the plan hash, because another run's ledger writes must
/// not invalidate a plan someone approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerToken(String);

impl LedgerToken {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the ledger records about a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The object's physical id; `None` for a write-ahead intent whose create has not
    /// returned.
    pub physical: Option<PhysicalId>,
    /// Whether the create is confirmed (the record is not only an intent).
    pub confirmed: bool,
    /// The nodes this one needed when it was created: the edges deletion follows, in reverse.
    pub deps: BTreeSet<NodePath>,
}

/// An object that is there, as the provider reports it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveState {
    pub live: Live,
    /// The provider's answer for this object (`Provider::deletability`).
    pub deletability: Deletability,
    /// The object is in a failed state and is updated toward the node even if its modelled
    /// fields match.
    pub failed: bool,
}

/// Everything known about one node path: the object (if there is one) and the record (if
/// there is one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub path: NodePath,
    pub kind: Kind,
    /// The provider's answer for the kind (`Provider::state_class`). A stateful kind is kept
    /// when its node is removed.
    pub state_class: StateClass,
    pub live: Option<LiveState>,
    pub record: Option<Record>,
}

/// The observed state of a run: one [`Seen`] per path the caller looked at, which is every
/// node of the lowered graph (wanted or not) and every path the ledger records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    token: LedgerToken,
    entries: BTreeMap<NodePath, Seen>,
    outputs: BTreeMap<(String, String), String>,
}

impl Observed {
    pub fn new(token: LedgerToken) -> Self {
        Self {
            token,
            entries: BTreeMap::new(),
            outputs: BTreeMap::new(),
        }
    }

    /// Records the value an applied resource produced for `output` (`URL`, `HOST`), as the
    /// ledger holds it. A node that references it is compared with the value filled in; a
    /// reference with no recorded value is not yet known.
    #[must_use]
    pub fn with_output(
        mut self,
        resource: impl Into<String>,
        output: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.outputs
            .insert((resource.into(), output.into()), value.into());
        self
    }

    /// Adds what is known about a path, replacing an earlier entry for the same path.
    #[must_use]
    pub fn with(mut self, seen: Seen) -> Self {
        self.entries.insert(seen.path.clone(), seen);
        self
    }

    pub fn token(&self) -> &LedgerToken {
        &self.token
    }

    pub(crate) fn output(&self, resource: &str, output: &str) -> Option<&str> {
        self.outputs
            .get(&(resource.to_owned(), output.to_owned()))
            .map(String::as_str)
    }

    pub(crate) fn entries(&self) -> &BTreeMap<NodePath, Seen> {
        &self.entries
    }
}
