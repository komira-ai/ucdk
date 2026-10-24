// SPDX-License-Identifier: Apache-2.0
//! What a provider reports back: observations, results of mutations, owned objects.

use crate::ids::{Digest, OwnerLabel, PhysicalId};
use crate::node::Kind;
use crate::stamp::{Stamp, StampEvidence};

/// A live object as the provider reads it, live from the cloud, never from a cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub physical: PhysicalId,
    /// The human name without any stamp encoding.
    pub name: String,
    pub stamp: StampEvidence,
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

/// The objects of one owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerScope {
    pub owner: OwnerLabel,
}

impl OwnerScope {
    pub fn new(owner: OwnerLabel) -> Self {
        Self { owner }
    }
}

/// A live object that carries a stamp of the scope's owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedObject {
    pub kind: Kind,
    pub physical: PhysicalId,
    pub name: String,
    pub stamp: Stamp,
}
