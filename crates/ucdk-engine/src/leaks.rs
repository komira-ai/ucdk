// SPDX-License-Identifier: Apache-2.0
//! The leak sweep: the objects of this cell that carry our stamp and that the ledger does not
//! account for. It reads and reports; a stamp never authorises a delete.

use ucdk_ledger::Record;
use ucdk_provider::{CellScope, OwnedObject, Stamp};

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

    fn sweep(&mut self) -> Result<Vec<Leak>, EngineError> {
        let snapshot = self.ledger.read()?;
        let records: Vec<&Record> = snapshot.records.values().collect();
        let listed = self
            .provider
            .list_stamped(&CellScope::new(self.cell.clone()))?;
        let mut leaks: Vec<Leak> = listed
            .into_iter()
            .filter(|object| !self.accounted(object, &records))
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

    /// Whether some record stands for the object: one that names its physical id, or an intent
    /// that names none yet and is for the node the stamp names (a create whose answer was lost;
    /// the next run takes the object up).
    fn accounted(&self, object: &OwnedObject, records: &[&Record]) -> bool {
        records.iter().any(|record| match &record.physical {
            Some(physical) => *physical == object.physical,
            None => Stamp::for_node(&self.cell, &record.path).same_identity(&object.stamp),
        })
    }
}
