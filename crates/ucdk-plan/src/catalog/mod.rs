// SPDX-License-Identifier: Apache-2.0
//! The catalog: checking a `ucdk.resource.v1` list and lowering it to nodes.
//!
//! # Lowering
//!
//! Lowering is driven by the descriptors: a [`Catalog`] is read from the resource list message
//! and the options on it (see `resource.proto` and section 6.4 of the architecture document),
//! and no code here names a type. Each resource lowers to a fixed set of nodes, one per role of
//! its type, with the id `<resource>/<role>`. For the two types of `ucdk.resource.v1`:
//!
//! | type | roles |
//! |---|---|
//! | `service` | `run` (always wanted), `public` (wanted when the exposure is `public`) |
//! | `job` | `run` (always wanted), `schedule` (wanted when the trigger is a schedule) |
//!
//! Every role is emitted for every resource. A role the file turns off is emitted with
//! `wanted = false`: planning deletes an object left by it, so a role cannot be turned off
//! and forgotten (closed-world removal). The `public` and `schedule` nodes depend on their
//! resource's `run`; a `run` depends on the `run` of each resource it references (a `Uses`
//! target or a value reference). A value reference is not a field of the node: it lowers to a
//! [`ValueRef`] on the node (the field name, the producing resource, the output), because the
//! value exists only once the target is applied. [`PlanNode::bound`] gives the node with the
//! values filled in.

mod descriptor;
mod finding;
mod lower;
mod read;
mod render;

pub use descriptor::{Catalog, CatalogError};
pub use finding::{Finding, FindingKind};
pub use lower::{Lowered, PlanNode, ROLE_RUN, ValueRef};
pub use render::{ROLE_PUBLIC, ROLE_SCHEDULE};

use prost::Message as _;
use prost_reflect::DynamicMessage;
use ucdk_proto::v1::{Portability, ResourceList};

impl Catalog {
    /// Checks a resource list and returns every finding, not only the first. An empty result
    /// means [`Catalog::lower`] succeeds. A reference to a resource that is not in the list and
    /// a cycle of references are refused here. `list` must be an instance of the catalog's list
    /// message ([`Catalog::new_list`]).
    pub fn validate(&self, list: &DynamicMessage) -> Vec<Finding> {
        self.validate_with_portability(list, |name| {
            self.portability(name).unwrap_or(Portability::Portable)
        })
    }

    /// [`Catalog::validate`] for a catalog that marks types differently: `portability` says how
    /// portable each type is, by name. A reference to a resource of a `CLOUD_BOUND` type is
    /// refused: only the one cloud that hosts it has the value, so no other node may be bound
    /// to it.
    pub fn validate_with_portability(
        &self,
        list: &DynamicMessage,
        portability: impl Fn(&str) -> Portability,
    ) -> Vec<Finding> {
        read::read(self, list, portability)
            .err()
            .unwrap_or_default()
    }

    /// Lowers a resource list to its nodes in dependency order, or returns the findings of
    /// [`Catalog::validate`].
    pub fn lower(&self, list: &DynamicMessage) -> Result<Lowered, Vec<Finding>> {
        read::read(self, list, |name| {
            self.portability(name).unwrap_or(Portability::Portable)
        })
        .map(lower::lower_specs)
    }
}

/// The `ucdk.resource.v1` list as the catalog reads it.
fn dynamic(list: &ResourceList) -> DynamicMessage {
    DynamicMessage::decode(
        Catalog::builtin().list.clone(),
        list.encode_to_vec().as_slice(),
    )
    .expect("a generated list decodes as its own descriptor")
}

/// Checks a resource list of the built-in catalog; see [`Catalog::validate`].
pub fn validate(list: &ResourceList) -> Vec<Finding> {
    Catalog::builtin().validate(&dynamic(list))
}

/// [`validate`] for a catalog that marks types differently; see
/// [`Catalog::validate_with_portability`]. `portability` is asked by type name (`service`,
/// `job`).
pub fn validate_with_portability(
    list: &ResourceList,
    portability: impl Fn(&str) -> Portability,
) -> Vec<Finding> {
    Catalog::builtin().validate_with_portability(&dynamic(list), portability)
}

/// Lowers a resource list of the built-in catalog to its nodes in dependency order, or returns
/// the findings of [`validate`].
pub fn lower(list: &ResourceList) -> Result<Lowered, Vec<Finding>> {
    Catalog::builtin().lower(&dynamic(list))
}
