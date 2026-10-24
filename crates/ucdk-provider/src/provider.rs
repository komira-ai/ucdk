// SPDX-License-Identifier: Apache-2.0
//! The provider trait.

use crate::error::ProviderError;
use crate::ids::PhysicalId;
use crate::node::{Kind, Node};
use crate::observe::{
    Created, Deletability, Live, Observation, OwnedObject, OwnerScope, StateClass, Updated,
};
use crate::stamp::{Stamp, StampMode};

/// One cloud, as the engine sees it. Synchronous: a real provider blocks inside its own
/// calls. Every method that touches the cloud takes `&mut self`, so one caller at a time
/// owns the provider (and a fake can count and log reads).
///
/// Contract, checked by [`crate::kit`]:
/// - `create` writes the stamp in the create call; an existing object of that kind and name
///   (ours or not) is `Refused(AlreadyExists)` and is left as it was;
/// - `update` of an object that is not there is `Refused(NotFound)`;
/// - `delete` of an object that is not there succeeds;
/// - `observe` and `list_owned` read the cloud as it is now;
/// - `delete` removes the object it names and no other;
/// - `list_owned` returns the objects of the scope's owner and no others;
/// - `observe` reports the human name of the object, never a name with the stamp encoded in it;
/// - `stamp_mode` and `state_class` of a kind do not change while objects exist, and an
///   object holding nothing the catalog does not model (`Live::unmanaged` empty) is `Deletable`;
/// - a create over a foreign object of the same name is refused as `AlreadyExists` or lands
///   beside it (a provider that puts the stamp in the physical name); either way the foreign
///   object is left as it was;
/// - `create` with a non-empty `Stamp::extra` on a kind whose `stamp_mode` does not carry extra
///   labels is `Refused(Unsupported)` and changes nothing; on a kind that does, `observe` and
///   `list_owned` return the extra labels exactly as written (a provider never drops them);
/// - `observe` with a hint finds the object by its physical id, whatever it is now called.
pub trait Provider: Send {
    /// Whether the kind's objects carry data. A stateful kind is kept by default.
    fn state_class(&self, kind: &Kind) -> StateClass;

    /// Whether `live` may be deleted now.
    fn deletability(&self, kind: &Kind, live: &Live) -> Deletability;

    /// Where the kind carries its stamp.
    fn stamp_mode(&self, kind: &Kind) -> StampMode;

    /// Reads the object for `node`: by `hint` when the engine knows the physical id, else by
    /// the node's name.
    fn observe(
        &mut self,
        node: &Node,
        hint: Option<&PhysicalId>,
    ) -> Result<Observation, ProviderError>;

    /// Creates the object, carrying `stamp` from its first moment.
    fn create(&mut self, node: &Node, stamp: &Stamp) -> Result<Created, ProviderError>;

    /// Converges the modelled fields of the live object to `node`'s. The stamp is kept.
    fn update(&mut self, node: &Node, live: &Live) -> Result<Updated, ProviderError>;

    /// Deletes the object; one that is already gone is success.
    fn delete(&mut self, kind: &Kind, physical: &PhysicalId) -> Result<(), ProviderError>;

    /// Every object that carries a stamp of the scope's owner.
    fn list_owned(&mut self, scope: &OwnerScope) -> Result<Vec<OwnedObject>, ProviderError>;
}
