// SPDX-License-Identifier: Apache-2.0
//! What a provider reports back: observations, results of mutations, owned objects.

use crate::cell::CellIdentity;
use crate::ids::{Digest, OwnerLabel, PhysicalId};
use crate::node::Kind;
use crate::stamp::{Stamp, StampEvidence};

/// Whether a live object is working, as the provider reads it now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The object is in service.
    Ready,
    /// The object is still coming up or changing; it may become ready or failed on its own.
    /// A run makes no write to it and does not confirm its record.
    Converging,
    /// The object is in a failed state. Its modelled fields may match the node's, and it is
    /// still not what the node asks for, so a run updates it.
    Failed,
}

/// A live object as the provider reads it, live from the cloud, never from a cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub physical: PhysicalId,
    /// The human name without any stamp encoding.
    pub name: String,
    pub stamp: StampEvidence,
    /// Whether the object is working: read from the cloud with the rest of the object, never
    /// assumed from its digest.
    pub status: Status,
    /// Digest of the kind and the modelled fields as they are now.
    pub digest: Digest,
    /// Names of fields the object has that the catalog does not model: reported, never
    /// compared, never converged.
    pub unmanaged: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    Absent,
    Present(Live),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub physical: PhysicalId,
    pub digest: Digest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Updated {
    pub digest: Digest,
}

/// Whether a kind holds data the engine keeps by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateClass {
    Stateless,
    Stateful,
}

/// Whether the provider lets a live object be deleted. Derived from live state, never
/// authored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deletability {
    Deletable,
    Undeletable { reason: String },
}

/// The objects of one owner in one cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerScope {
    pub cell: CellIdentity,
    pub owner: OwnerLabel,
}

impl OwnerScope {
    pub fn new(cell: CellIdentity, owner: OwnerLabel) -> Self {
        Self { cell, owner }
    }
}

/// Every object of one cell, whoever in the cell owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellScope {
    pub cell: CellIdentity,
}

impl CellScope {
    pub fn new(cell: CellIdentity) -> Self {
        Self { cell }
    }
}

/// A live object that carries a stamp of the scope's cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedObject {
    pub kind: Kind,
    pub physical: PhysicalId,
    pub name: String,
    pub stamp: Stamp,
}
