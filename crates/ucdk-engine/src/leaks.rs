// SPDX-License-Identifier: Apache-2.0
//! The leak sweep: the objects of this cell that carry our stamp and that the ledger does not
//! account for. It reads and reports; a stamp never authorises a delete.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_ledger::Record;
use ucdk_provider::{CellScope, NodePath, OwnedObject, PhysicalId, Stamp};

use crate::engine::Engine;
use crate::error::EngineError;
use crate::seam::{Failure, Leak, Outcome, Report};

impl Engine<'_> {
    /// Lists the cell's stamped objects through the provider and reads the ledger, and nothing
    /// else: no ledger write and no provider call that mutates.
    pub(crate) fn leaks_of(&mut self) -> Outcome {
        match self.sweep() {
            Ok(leaks) => Outcome::Leaks(leaks),
            // A sweep changes nothing, so a failure to read is never a partial run.
            Err(error) => Outcome::Failed(Failure {
                report: Report::default(),
                at: None,
                error,
            }),
        }
    }

    /// The last step of an apply: a sweep whose answer joins the report. A sweep that cannot
    /// read is recorded in the report and does not fail the apply that already finished.
    pub(crate) fn sweep_into(&mut self, report: &mut Report) {
        match self.sweep() {
            Ok(leaks) => report.leaks = leaks,
            Err(error) => report.leak_sweep_failed = Some(error),
        }
    }

    /// The ledger is read before the provider is listed and again after, and an object is
    /// accounted for if either read accounts for it. A run that writes in between is not
    /// halted (the sweep takes no lease): an object it creates after the first read has its
    /// intent recorded before it exists, so the second read names it; an object it deletes
    /// before the list is read was named by the first. The sweep reports only what neither
    /// reading explains.
    fn sweep(&mut self) -> Result<Vec<Leak>, EngineError> {
        let first = self.ledger.read()?;
        let listed = self
            .provider
            .list_stamped(&CellScope::new(self.cell.clone()))?;
        let second = self.ledger.read()?;
        let accounted = self.accounted(&listed, [&first.records, &second.records]);
        let mut leaks: Vec<Leak> = listed
            .into_iter()
            .filter(|object| !accounted.contains(&object.physical))
            .map(|object| Leak {
                kind: object.kind,
                physical: object.physical,
                name: object.name,
                owner: object.stamp.owner,
                node: object.stamp.node,
            })
            .collect();
        leaks.sort_by(|a, b| a.physical.cmp(&b.physical));
        Ok(leaks)
    }

    /// The physical ids of `listed` that the records of either reading stand for. A record
    /// that names a physical id stands for that object. An intent that names none yet (a
    /// create whose answer was lost; the next run takes the object up) stands for one object
    /// stamped for its node, the lowest physical id, so a second stamped object for the node is
    /// a duplicate and not hidden by it.
    fn accounted(
        &self,
        listed: &[OwnedObject],
        readings: [&BTreeMap<NodePath, Record>; 2],
    ) -> BTreeSet<PhysicalId> {
        let named: BTreeSet<PhysicalId> = readings
            .iter()
            .flat_map(|records| records.values())
            .filter_map(|record| record.physical.clone())
            .collect();
        // The later reading's record of a path replaces the earlier one's, for the intents.
        let mut latest: BTreeMap<&NodePath, &Record> = BTreeMap::new();
        for records in readings {
            latest.extend(records.iter());
        }
        let mut accounted = named;
        for record in latest.values().filter(|r| r.physical.is_none()) {
            let stamp = Stamp::for_node(&self.cell, &record.path);
            let stands_for = listed
                .iter()
                .filter(|o| !accounted.contains(&o.physical) && stamp.same_identity(&o.stamp))
                .map(|o| &o.physical)
                .min();
            if let Some(physical) = stands_for.cloned() {
                accounted.insert(physical);
            }
        }
        accounted
    }
}
