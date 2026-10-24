// SPDX-License-Identifier: Apache-2.0
//! Lowering: the nodes of each resource, one per role of its type.

use std::collections::BTreeSet;

use ucdk_provider::{DesiredField, Kind, Node, NodePath};

use super::read::Spec;

/// The role every resource has: the object that runs the container.
pub const ROLE_RUN: &str = "run";

/// One node of the lowered graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanNode {
    pub node: Node,
    /// The nodes that must exist before this one is created.
    pub deps: BTreeSet<NodePath>,
    /// False for a role the file turns off: the node stays in the graph so that an object
    /// left behind by the role's removal is found and deleted (closed-world removal).
    pub wanted: bool,
}

/// The nodes of a resource list, in dependency order: a node comes after every node it needs.
/// Every resource contributes all the roles of its type, wanted or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lowered {
    nodes: Vec<PlanNode>,
}

impl Lowered {
    pub fn nodes(&self) -> &[PlanNode] {
        &self.nodes
    }
}

fn path(id: &str, role: &str) -> NodePath {
    NodePath::new(&format!("{id}/{role}"))
        .expect("a resource id was checked as one segment and a role is a fixed word")
}

fn node(id: &str, role: &str, kind: Kind, fields: Vec<DesiredField>) -> Node {
    // The name is the path with `-` for `/`. A role never contains `-`, so the text after the
    // last `-` is the role and two nodes never share a name.
    Node::new(path(id, role), kind, format!("{id}-{role}"), fields)
        .expect("field names are distinct: each kind of field has its own prefix")
}

pub fn lower_specs(specs: Vec<Spec>) -> Lowered {
    let mut nodes = Vec::with_capacity(specs.len() * 2);
    for spec in specs {
        let run = path(&spec.id, ROLE_RUN);
        let deps = spec
            .run_deps
            .iter()
            .map(|dep| path(dep, ROLE_RUN))
            .collect();
        nodes.push(PlanNode {
            node: node(&spec.id, ROLE_RUN, spec.kind, spec.run_fields),
            deps,
            wanted: true,
        });
        nodes.push(PlanNode {
            node: node(
                &spec.id,
                spec.extra.role,
                spec.extra.kind,
                spec.extra.fields,
            ),
            deps: BTreeSet::from([run]),
            wanted: spec.extra.wanted,
        });
    }
    Lowered { nodes }
}
