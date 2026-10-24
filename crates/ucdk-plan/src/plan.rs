// SPDX-License-Identifier: Apache-2.0
//! The plan: what to create, update and delete to make the observed state match the lowered
//! nodes. It is a function of its arguments and changes none of them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ucdk_provider::{
    Deletability, Kind, Live, NodePath, OwnerLabel, PhysicalId, Stamp, StampEvidence, StateClass,
};

use crate::catalog::{Lowered, PlanNode, ROLE_RUN};
use crate::graph;
use crate::hash::{self, PlanHash};
use crate::observed::{Observed, Record, Seen};

/// Why a node that is not deleted is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Retained {
    /// The node's kind holds data, so removing the node keeps the object.
    Keep,
    /// The provider says the object cannot be deleted now.
    Undeletable(String),
    /// The file no longer names the node's owner: a run never deletes a whole owner's objects.
    OwnerNotNamed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Create,
    /// Converge the modelled fields of the object in place.
    Update,
    Noop,
    /// Delete the object with this physical id.
    Delete {
        physical: PhysicalId,
    },
    /// The object is already gone: drop the record, with no provider call.
    Reap,
    Retain(Retained),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub path: NodePath,
    pub kind: Kind,
    pub action: Action,
}

/// The creates, updates and deletes of a run, in the order to carry them out: nodes the file
/// wants in dependency order, then removals in reverse dependency order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    steps: Vec<Step>,
    hash: PlanHash,
}

impl Plan {
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn hash(&self) -> &PlanHash {
        &self.hash
    }
}

/// Why an object is not ours to plan around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictKind {
    /// An object is there that carries no stamp and has no record.
    Foreign,
    /// The object carries our stamp but the ledger has no record of it.
    StampWithoutRecord,
    /// The record names a different object than the one that carries the stamp.
    PhysicalIdMismatch,
    /// The ledger has a record but the object has lost its stamp.
    StampStripped,
    /// The stamp is another owner's, or another node's.
    OtherOwner,
    /// The provider recognises another tool's ownership marker on the object.
    OtherTool(String),
    /// A kept object that was confirmed is gone: it is not silently recreated.
    VanishedRetained,
    /// The ledger's recorded edges form a cycle, so removals cannot be ordered.
    RecordedCycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub path: NodePath,
    pub why: ConflictKind,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = match &self.why {
            ConflictKind::Foreign => "an object with no stamp is there".to_owned(),
            ConflictKind::StampWithoutRecord => {
                "the object has our stamp but there is no record".to_owned()
            }
            ConflictKind::PhysicalIdMismatch => {
                "the record names a different object than the stamped one".to_owned()
            }
            ConflictKind::StampStripped => "the recorded object has lost its stamp".to_owned(),
            ConflictKind::OtherOwner => "the stamp belongs to another owner or node".to_owned(),
            ConflictKind::OtherTool(marker) => format!("another tool owns it ({marker})"),
            ConflictKind::VanishedRetained => {
                "a kept object is gone and is not recreated unasked".to_owned()
            }
            ConflictKind::RecordedCycle => "the recorded dependencies form a cycle".to_owned(),
        };
        write!(f, "{}: {why}", self.path)
    }
}

/// A run refused whole, before any step: every offender is listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub conflicts: Vec<Conflict>,
}

/// Whether the object of `seen` is ours: stamped for this node and recorded.
fn classify(path: &NodePath, live: &Live, record: Option<&Record>) -> Option<ConflictKind> {
    match &live.stamp {
        StampEvidence::OtherTool { marker } => Some(ConflictKind::OtherTool(marker.clone())),
        StampEvidence::Absent if record.is_some() => Some(ConflictKind::StampStripped),
        StampEvidence::Absent => Some(ConflictKind::Foreign),
        StampEvidence::Present(stamp) if !stamp.same_identity(&Stamp::for_node(path)) => {
            Some(ConflictKind::OtherOwner)
        }
        StampEvidence::Present(_) => match record {
            None => Some(ConflictKind::StampWithoutRecord),
            Some(r) if r.physical.as_ref().is_some_and(|p| *p != live.physical) => {
                Some(ConflictKind::PhysicalIdMismatch)
            }
            Some(_) => None,
        },
    }
}

fn conflict_of(seen: &Seen, wanted: bool) -> Option<ConflictKind> {
    match &seen.live {
        Some(state) => classify(&seen.path, &state.live, seen.record.as_ref()),
        None => {
            let confirmed = seen.record.as_ref().is_some_and(|r| r.confirmed);
            let kept = seen.state_class == StateClass::Stateful;
            (wanted && confirmed && kept).then_some(ConflictKind::VanishedRetained)
        }
    }
}

/// What to do about a wanted node. `settled` says whether the resource that produced a
/// referenced output is itself left alone by this run: only then is its recorded output
/// the value the object will see. Otherwise the value is known after apply, and an object
/// that is there is updated.
fn forward(
    node: &PlanNode,
    seen: Option<&Seen>,
    observed: &Observed,
    settled: &BTreeSet<String>,
) -> Step {
    let bound = node.bound(|resource, output| {
        settled
            .contains(resource)
            .then(|| observed.output(resource, output))
            .flatten()
    });
    let action = match (seen.and_then(|s| s.live.as_ref()), bound) {
        (None, _) => Action::Create,
        (Some(_), Err(_)) => Action::Update,
        (Some(state), Ok(bound)) if state.failed || state.live.digest != bound.digest() => {
            Action::Update
        }
        (Some(_), Ok(_)) => Action::Noop,
    };
    Step {
        path: node.node.path().clone(),
        kind: node.node.kind().clone(),
        action,
    }
}

fn removal(seen: &Seen, owners: &BTreeSet<OwnerLabel>) -> Step {
    let action = match &seen.live {
        None => Action::Reap,
        Some(_) if !owners.contains(&seen.path.owner()) => Action::Retain(Retained::OwnerNotNamed),
        Some(state) => match &state.deletability {
            Deletability::Undeletable { reason } => {
                Action::Retain(Retained::Undeletable(reason.clone()))
            }
            Deletability::Deletable if seen.state_class == StateClass::Stateful => {
                Action::Retain(Retained::Keep)
            }
            Deletability::Deletable => Action::Delete {
                physical: state.live.physical.clone(),
            },
        },
    };
    Step {
        path: seen.path.clone(),
        kind: seen.kind.clone(),
        action,
    }
}

/// Plans a run: compares `desired` with `observed` and returns the steps, or refuses the
/// whole run.
///
/// - A node the file wants with no object is created; one whose object differs from the node
///   in a modelled field, or is failed, is updated in place; one that matches is a no-op.
/// - A path the file does not want (a role turned off, a resource removed) with an object
///   that is ours is deleted, unless its kind holds data (kept), the provider says it cannot
///   be deleted, or the file no longer names its owner at all (retained, never deleted by a
///   run). One whose object is already gone is reaped.
/// - An object is ours only if it carries this node's stamp and the ledger records it.
///   Anything else on a path that was looked at, wanted or not, refuses the whole run and
///   lists every such path. So does a confirmed kept object that has vanished.
///
/// `provider_version` is reserved for the plan hash, so a provider upgrade can invalidate an
/// approved plan; pass `None` until a provider reports one.
///
/// Nothing is called and nothing is changed: `desired` and `observed` are only read.
pub fn plan(
    desired: &Lowered,
    observed: &Observed,
    provider_version: Option<&str>,
) -> Result<Plan, Refused> {
    let wanted: BTreeMap<&NodePath, &PlanNode> = desired
        .nodes()
        .iter()
        .filter(|n| n.wanted)
        .map(|n| (n.node.path(), n))
        .collect();
    let owners: BTreeSet<OwnerLabel> = desired
        .nodes()
        .iter()
        .map(|n| n.node.path().owner())
        .collect();

    let conflicts: Vec<Conflict> = observed
        .entries()
        .values()
        .filter_map(|seen| {
            conflict_of(seen, wanted.contains_key(&seen.path)).map(|why| Conflict {
                path: seen.path.clone(),
                why,
            })
        })
        .collect();
    if !conflicts.is_empty() {
        return Err(Refused { conflicts });
    }

    // Nodes come in dependency order, so a producer's step is known before its consumers'. A
    // resource is settled when its `run` node is a no-op; its recorded outputs are then current.
    let mut steps: Vec<Step> = Vec::new();
    let mut settled: BTreeSet<String> = BTreeSet::new();
    for n in desired.nodes().iter().filter(|n| n.wanted) {
        let step = forward(n, observed.entries().get(n.node.path()), observed, &settled);
        if n.node.path().as_str().ends_with(&format!("/{ROLE_RUN}")) && step.action == Action::Noop
        {
            settled.insert(n.node.path().owner().as_str().to_owned());
        }
        steps.push(step);
    }

    // Removals: whatever was looked at that the file does not want and that is there or
    // recorded. They are ordered by the edges the ledger recorded, dependents first.
    let gone: Vec<&Seen> = observed
        .entries()
        .values()
        .filter(|s| !wanted.contains_key(&s.path) && (s.live.is_some() || s.record.is_some()))
        .collect();
    let edges: BTreeMap<NodePath, BTreeSet<NodePath>> = gone
        .iter()
        .map(|s| {
            let needs = s
                .record
                .iter()
                .flat_map(|r| r.deps.iter().cloned())
                .collect();
            (s.path.clone(), needs)
        })
        .collect();
    let order = graph::order(&edges).map_err(|stuck| Refused {
        conflicts: stuck
            .into_iter()
            .map(|path| Conflict {
                path,
                why: ConflictKind::RecordedCycle,
            })
            .collect(),
    })?;
    let mut removals: BTreeMap<NodePath, Step> = gone
        .iter()
        .map(|s| (s.path.clone(), removal(s, &owners)))
        .collect();
    for path in order.iter().rev() {
        steps.extend(removals.remove(path));
    }

    let hash = hash::compute(desired, &steps, provider_version);
    Ok(Plan { steps, hash })
}
