// SPDX-License-Identifier: Apache-2.0
//! The catalog: checking a `ucdk.resource.v1` list and lowering it to nodes.
//!
//! # Lowering
//!
//! Each resource lowers to a fixed set of nodes, one per role of its type, with the id
//! `<resource>/<role>`:
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

mod finding;
mod lower;
mod read;
mod render;

pub use finding::{Finding, FindingKind};
pub use lower::{Lowered, PlanNode, ROLE_RUN, ValueRef};
pub use render::{ROLE_PUBLIC, ROLE_SCHEDULE};

use ucdk_proto::v1::{Portability, ResourceList};

/// Checks a resource list and returns every finding, not only the first. An empty result
/// means [`lower`] succeeds. A reference to a resource that is not in the list and a cycle of
/// references are refused here.
pub fn validate(list: &ResourceList) -> Vec<Finding> {
    validate_with_portability(list, read::catalog_portability)
}

/// [`validate`] for a catalog that marks types differently: `portability` says how portable
/// each type is, by name (`service`, `job`). A reference to a resource of a `CLOUD_BOUND` type
/// is refused: only the one cloud that hosts it has the value, so no other node may be bound
/// to it. Both v1 types are `PORTABLE`, so `validate` never reports it.
pub fn validate_with_portability(
    list: &ResourceList,
    portability: impl Fn(&str) -> Portability,
) -> Vec<Finding> {
    read::read(list, portability).err().unwrap_or_default()
}

/// Lowers a resource list to its nodes in dependency order, or returns the findings of
/// [`validate`].
pub fn lower(list: &ResourceList) -> Result<Lowered, Vec<Finding>> {
    read::read(list, read::catalog_portability).map(lower::lower_specs)
}
