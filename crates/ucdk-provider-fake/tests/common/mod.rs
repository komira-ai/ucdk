// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]
//! Helpers shared by the fake's tests.

use ucdk_provider::{CellIdentity, DesiredField, FieldValue, Kind, Node, NodePath, Stamp, kit};

/// The cell the tests work in.
pub fn cell() -> CellIdentity {
    kit::cell()
}

pub fn node(path: &str, kind: Kind, name: &str, port: i64) -> Node {
    Node::new(
        NodePath::new(path).expect("valid path"),
        kind,
        name,
        vec![
            DesiredField::new("port", FieldValue::Int(port)),
            DesiredField::new("label", FieldValue::Text("a".to_owned())),
            DesiredField::new("public", FieldValue::Bool(false)),
        ],
    )
    .expect("distinct fields")
}

pub fn service(path: &str, name: &str) -> Node {
    node(path, Kind::service(), name, 80)
}

pub fn job(path: &str, name: &str) -> Node {
    node(path, Kind::job(), name, 80)
}

pub fn stamp(node: &Node) -> Stamp {
    Stamp::for_node(&cell(), node.path())
}
