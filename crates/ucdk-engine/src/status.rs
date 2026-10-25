// SPDX-License-Identifier: Apache-2.0
//! Status: what the ledger records and what the provider shows, side by side, with no write.

use std::collections::BTreeMap;

use ucdk_ledger::{Record, State};
use ucdk_plan::Lowered;
use ucdk_provider::{Node, NodePath, Observation};

use crate::engine::{Engine, node_of};
use crate::error::EngineError;
use crate::seam::{Failure, ObjectStatus, Outcome, Standing};

impl Engine<'_> {
    /// Reads the ledger, then reads each object of `desired` and each recorded object through
    /// the provider. It calls `Ledger::read` and `Provider::observe` and nothing else, so it
    /// can run beside a run that is changing the world: it reports what it saw.
    pub(crate) fn status_of(&mut self, desired: &Lowered) -> Outcome {
        if let Some(refused) = self.refuse_unhosted(desired) {
            return refused;
        }
        match self.status_entries(desired) {
            Ok(entries) => Outcome::Status(entries),
            // Nothing is changed by a status, so a failure to read it is never a partial run.
            Err(error) => Outcome::Failed(Failure {
                report: crate::seam::Report::default(),
                at: None,
                error,
            }),
        }
    }

    fn status_entries(&mut self, desired: &Lowered) -> Result<Vec<ObjectStatus>, EngineError> {
        let snapshot = self.ledger.read()?;
        let mut subjects: BTreeMap<NodePath, Node> = snapshot
            .records
            .values()
            .map(|r| (r.path.clone(), node_of(r)))
            .collect();
        for n in desired.nodes().iter().filter(|n| n.wanted) {
            subjects.insert(n.node.path().clone(), n.node.clone());
        }
        let mut entries = Vec::with_capacity(subjects.len());
        for (path, node) in subjects {
            let record = snapshot.records.get(&path);
            let live = self.observe_node(&node, record)?;
            let (status, standing) = match &live {
                Observation::Absent => (
                    None,
                    if record.is_some() {
                        Standing::Missing
                    } else {
                        Standing::NotApplied
                    },
                ),
                Observation::Present(live) => (
                    Some(live.status),
                    standing_of(self.ours(live, &path, record), record),
                ),
            };
            entries.push(ObjectStatus {
                path,
                kind: node.kind().clone(),
                record: record.map(|r| r.state),
                live: status,
                standing,
                outputs: record.map(|r| r.outputs.clone()).unwrap_or_default(),
            });
        }
        Ok(entries)
    }
}

/// How a present object stands against its record: a confirmed record of an object that is
/// ours agrees; any other record of one that is ours is a run still to settle; an object that
/// is not ours, or has no record, is unclaimed.
fn standing_of(ours: bool, record: Option<&Record>) -> Standing {
    match record {
        Some(r) if ours && r.state == State::Confirmed => Standing::Agrees,
        Some(_) if ours => Standing::Pending,
        _ => Standing::Unclaimed,
    }
}
