// SPDX-License-Identifier: Apache-2.0
//! Lowering: the nodes of each resource, one per role of its type.

use std::collections::BTreeSet;

use ucdk_proto::v1::Output;
use ucdk_provider::{DesiredField, FieldValue, Kind, Node, NodePath};

use super::read::Spec;

/// The role every resource has: the object that runs the container.
pub const ROLE_RUN: &str = "run";

/// A field of a node whose value is an output of another resource: a value that exists only
/// once that resource is applied. It is kept apart from the node's literal fields, so a
/// literal that happens to read like a reference is still only a literal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ValueRef {
    /// The name the field takes in the node once the value is known.
    pub field: String,
    /// The resource that produces the value.
    pub resource: String,
    /// The output of that resource.
    pub output: Output,
}

/// One node of the lowered graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanNode {
    /// The node with its literal fields; the fields of `refs` are not in it yet.
    pub node: Node,
    /// The fields whose value comes from another resource's output.
    pub refs: Vec<ValueRef>,
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

    /// The same nodes with none of them wanted: the desired state of a destroy. Planning
    /// against it removes what the file names and leaves everything else alone.
    #[must_use]
    pub fn unwanted(&self) -> Self {
        let nodes = self
            .nodes
            .iter()
            .map(|n| PlanNode {
                wanted: false,
                ..n.clone()
            })
            .collect();
        Self { nodes }
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

impl PlanNode {
    /// The node as it will be applied: its literal fields plus each referenced output, looked
    /// up with `output(resource, output_name)`. The refs whose output is not known are
    /// returned instead, so the caller can tell that the node's final value is not yet known.
    pub fn bound<'a>(
        &'a self,
        output: impl Fn(&str, &str) -> Option<&'a str>,
    ) -> Result<Node, Vec<&'a ValueRef>> {
        let mut fields = self.node.fields().to_vec();
        let mut pending = Vec::new();
        for r in &self.refs {
            match output(&r.resource, r.output.as_str_name()) {
                Some(text) => fields.push(DesiredField::new(
                    &r.field,
                    FieldValue::Text(text.to_owned()),
                )),
                None => pending.push(r),
            }
        }
        if !pending.is_empty() {
            return Err(pending);
        }
        Ok(Node::new(
            self.node.path().clone(),
            self.node.kind().clone(),
            self.node.name(),
            fields,
        )
        .expect("a ref field is named like no literal field: the catalog reads each env key once"))
    }
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
            refs: spec.run_refs,
            deps,
            wanted: true,
        });
        for role in spec.roles {
            nodes.push(PlanNode {
                node: node(&spec.id, &role.role, role.kind, role.fields),
                refs: Vec::new(),
                deps: BTreeSet::from([run.clone()]),
                wanted: role.wanted,
            });
        }
    }
    Lowered { nodes }
}
