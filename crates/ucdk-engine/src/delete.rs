// SPDX-License-Identifier: Apache-2.0
//! The removing steps: delete, reap, and rollback.

use ucdk_ledger::{Change, Origin, Record, State};
use ucdk_plan::Retained;
use ucdk_provider::{Deletability, NodePath, Observation, StateClass};

use crate::engine::{Engine, node_of, ours};
use crate::error::EngineError;
use crate::seam::Report;

/// What became of an object a removal was asked for.
enum Removal {
    Deleted,
    /// It was already gone: only the record was dropped.
    Gone,
    Kept(Retained),
}

/// An absent read drops a record only if the object was read after it was created. An intent
/// that names an object was created but never read back, so its absence may be a lagging read.
fn witnessed(record: &Record) -> Result<(), EngineError> {
    if record.state == State::Intent && record.physical.is_some() {
        return Err(EngineError::Unwitnessed(record.path.clone()));
    }
    Ok(())
}

impl Engine<'_> {
    /// Removes the object of the node at `path` and its record, if the object is ours (stamp and
    /// record), created by this tool, deletable and not data. The record goes only when a read
    /// shows the object gone.
    fn remove(&mut self, path: &NodePath) -> Result<Removal, EngineError> {
        let Some(record) = self.record_of(path)? else {
            return Err(EngineError::NotOurs(path.clone()));
        };
        let node = node_of(&record);
        let live = match self.observe_node(&node, Some(&record))? {
            Observation::Absent => {
                witnessed(&record)?;
                self.commit(&Change::Remove(path.clone()))?;
                return Ok(Removal::Gone);
            }
            Observation::Present(live) => live,
        };
        if !ours(&live, path, Some(&record)) {
            return Err(EngineError::NotOurs(path.clone()));
        }
        if record.origin != Origin::Created {
            return Ok(Removal::Kept(Retained::Undeletable(
                "the tool did not create it: it is released, not deleted".to_owned(),
            )));
        }
        if let Deletability::Undeletable { reason } =
            self.provider.deletability(&record.kind, &live)
        {
            return Ok(Removal::Kept(Retained::Undeletable(reason)));
        }
        if self.state_class(&record.kind) == StateClass::Stateful {
            return Ok(Removal::Kept(Retained::Keep));
        }
        self.commit(&Change::Put(record.clone().in_state(State::Reaping)))?;
        self.provider.delete(&record.kind, &live.physical)?;
        match self.observe_node(&node, Some(&record))? {
            Observation::Absent => {
                self.commit(&Change::Remove(path.clone()))?;
                Ok(Removal::Deleted)
            }
            Observation::Present(_) => Err(EngineError::StillPresent(path.clone())),
        }
    }

    pub(crate) fn delete_step(
        &mut self,
        path: &NodePath,
        report: &mut Report,
    ) -> Result<(), EngineError> {
        match self.remove(path)? {
            Removal::Deleted => report.deleted.push(path.clone()),
            Removal::Gone => report.reaped.push(path.clone()),
            Removal::Kept(why) => report.retained.push((path.clone(), why)),
        }
        Ok(())
    }

    /// Drops the record of an object the plan found gone, after reading again to be sure.
    pub(crate) fn reap_step(
        &mut self,
        path: &NodePath,
        report: &mut Report,
    ) -> Result<(), EngineError> {
        let Some(record) = self.record_of(path)? else {
            return Ok(());
        };
        match self.observe_node(&node_of(&record), Some(&record))? {
            Observation::Absent => {
                witnessed(&record)?;
                self.commit(&Change::Remove(path.clone()))?;
                report.reaped.push(path.clone());
                Ok(())
            }
            Observation::Present(_) => Err(EngineError::StillPresent(path.clone())),
        }
    }

    /// Deletes what this run created, newest first, under the rules of any delete. A failure
    /// is recorded and the rest is still tried.
    pub(crate) fn roll_back(&mut self, report: &mut Report) {
        let created: Vec<NodePath> = report.created.iter().rev().cloned().collect();
        for path in created {
            match self.remove(&path) {
                Ok(Removal::Deleted | Removal::Gone) => report.rolled_back.push(path),
                Ok(Removal::Kept(why)) => report.retained.push((path, why)),
                Err(error) => report.rollback_failed.push((path, error)),
            }
        }
    }
}
