// SPDX-License-Identifier: Apache-2.0
//! The provider trait and the model it works on.
//!
//! A provider turns a [`Node`] (one desired object: a kind, a name and its modelled fields)
//! into a live object in some cloud, and reports what it finds there. This crate is pure and
//! synchronous: it holds no I/O, no clock and no `HashMap`, so the same inputs give the same
//! results in the same order. Real providers implement [`Provider`] over their own SDK; the
//! in-memory fake in `ucdk-provider-fake` is the executable spec.
//!
//! The [`kit`] module is the contract every provider must pass. It is written against the
//! trait, so a provider is checked by the same functions as the fake.
//!
//! Ownership: every object a provider creates is born carrying a [`Stamp`], written in the
//! create call itself. A kind that cannot carry a tag encodes the stamp in the object's name
//! ([`StampMode::NameEncoded`]). Ownership is decided from the stamp, never from a name's
//! spelling alone.

pub mod cell;
pub mod error;
pub mod ids;
pub mod kit;
pub mod node;
pub mod observe;
pub mod provider;
pub mod stamp;

pub use cell::{CellIdentity, CellName, CellUid};
pub use error::{ProviderError, Refusal};
pub use ids::{
    Digest, ModelError, NODE_KEY_LEN, NodeKey, NodePath, OwnerLabel, PhysicalId, SEGMENT_MAX,
};
pub use node::{DesiredField, FieldValue, Kind, Node};
pub use observe::{
    CellScope, Created, Deletability, Live, Observation, OwnedObject, OwnerScope, StateClass,
    Status, Updated,
};
pub use provider::Provider;
pub use stamp::{NAME_SUFFIX_MAX, SCHEME, Stamp, StampEvidence, StampMode};
