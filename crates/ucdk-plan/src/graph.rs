// SPDX-License-Identifier: Apache-2.0
//! Dependency order.

use std::collections::{BTreeMap, BTreeSet};

/// Orders `deps` (each node, with the nodes it needs) so that a node comes after everything it
/// needs, by Kahn's algorithm. Of the nodes ready at once the smallest comes first, so the
/// order is the same for the same graph. A need that is not a key of `deps` is ignored.
///
/// A graph that cannot be ordered returns the nodes left over: those on a cycle (a node that
/// needs itself is one) and those that need such a node.
pub fn order<K: Ord + Clone>(deps: &BTreeMap<K, BTreeSet<K>>) -> Result<Vec<K>, Vec<K>> {
    let mut waiting: BTreeMap<&K, usize> = BTreeMap::new();
    let mut dependents: BTreeMap<&K, Vec<&K>> = BTreeMap::new();
    for (node, needs) in deps {
        let known: Vec<&K> = needs
            .iter()
            .filter_map(|need| deps.get_key_value(need).map(|(key, _)| key))
            .collect();
        waiting.insert(node, known.len());
        for need in known {
            dependents.entry(need).or_default().push(node);
        }
    }
    let mut ready: BTreeSet<&K> = waiting
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(node, _)| *node)
        .collect();
    let mut sorted = Vec::with_capacity(deps.len());
    while let Some(node) = ready.pop_first() {
        sorted.push(node.clone());
        for next in dependents.get(node).into_iter().flatten() {
            let count = waiting.entry(*next).or_default();
            *count -= 1;
            if *count == 0 {
                ready.insert(*next);
            }
        }
    }
    if sorted.len() == deps.len() {
        Ok(sorted)
    } else {
        Err(waiting
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(node, _)| node.clone())
            .collect())
    }
}
