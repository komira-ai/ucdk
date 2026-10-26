// SPDX-License-Identifier: Apache-2.0
//! Dependency order: Kahn's algorithm with ties broken by name.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_core::order;

fn graph(
    edges: &[(&'static str, &[&'static str])],
) -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    edges
        .iter()
        .map(|(node, needs)| (*node, needs.iter().copied().collect()))
        .collect()
}

#[test]
fn a_node_comes_after_everything_it_needs() {
    let deps = graph(&[("app", &["db", "cache"]), ("db", &[]), ("cache", &["db"])]);
    assert_eq!(order(&deps), Ok(vec!["db", "cache", "app"]));
}

#[test]
fn nodes_ready_together_come_out_in_name_order() {
    let deps = graph(&[("c", &[]), ("a", &[]), ("b", &[])]);
    assert_eq!(order(&deps), Ok(vec!["a", "b", "c"]));
}

#[test]
fn an_empty_graph_is_ordered() {
    assert_eq!(order::<&str>(&BTreeMap::new()), Ok(vec![]));
}

#[test]
fn a_need_outside_the_graph_is_ignored() {
    let deps = graph(&[("a", &["elsewhere"])]);
    assert_eq!(order(&deps), Ok(vec!["a"]));
}

#[test]
fn a_cycle_returns_its_members_and_what_depends_on_them() {
    let deps = graph(&[
        ("free", &[]),
        ("a", &["b"]),
        ("b", &["a"]),
        ("downstream", &["a"]),
    ]);
    assert_eq!(order(&deps), Err(vec!["a", "b", "downstream"]));
}

#[test]
fn a_node_that_needs_itself_is_a_cycle() {
    let deps = graph(&[("a", &["a"])]);
    assert_eq!(order(&deps), Err(vec!["a"]));
}
