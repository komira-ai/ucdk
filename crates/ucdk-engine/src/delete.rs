// SPDX-License-Identifier: Apache-2.0
//! The removing steps: delete, reap, and rollback.

use ucdk_ledger::{Change, Origin, Record, State};
use ucdk_plan::Retained;
use ucdk_provider::{Deletability, Node, NodePath, Observation, OwnerScope, StateClass};

use crate::engine::{Engine, node_of};
use crate::error::EngineError;
use crate::seam::Report;

/// What became of an object a removal was asked for.
enum Removal {
    Deleted,
    /// It was already gone: only the record was dropped.
    Gone,
    Kept(Retained),
}

impl Engine<'_> {
    /// Whether the read of an absent object may drop its record. Only an intent holds that
    /// back: it is written before a create or an update, so its call may have landed with the
    /// reply lost.
    ///
    /// - An intent that names an object was never read back: the absence may be a lagging
    ///   read, and the record stays until a run reads the object.
    /// - An intent that names none may belong to a create that did land, or to one that did not
    ///   (a reply lost to a timeout). It is dropped only on a definitive absence: besides the
    ///   read the caller made, two more passes, each reading the object by name and listing
    ///   the owner's objects, find nothing. Any read that fails (transient, in flight,
    ///   unknown) stops the run with the record kept.
    ///
    /// The passes cannot see past a lag longer than the reads they make. A reap then leaves an
    /// object that carries our stamp and has no record, which the next plan reports.
    fn witnessed(&mut self, record: &Record, node: &Node) -> Result<(), EngineError> {
        if record.state != State::Intent {
            return Ok(());
        }
        if record.physical.is_some() {
            return Err(EngineError::Unwitnessed(record.path.clone()));
        }
        for _ in 0..2 {
            if self.seen(record, node)? {
                return Err(EngineError::StillPresent(record.path.clone()));
            }
        }
        Ok(())
    }

    /// Whether either a read by name or a listing of the owner's objects shows the object of
    /// `record`.
    fn seen(&mut self, record: &Record, node: &Node) -> Result<bool, EngineError> {
        let scope = OwnerScope::new(self.cell.clone(), record.path.owner());
        let listed = self
            .provider
            .list_owned(&scope)?
            .iter()
            .any(|o| o.kind == record.kind && o.name == record.name);
        Ok(listed
            || matches!(
                self.observe_node(node, Some(record))?,
                Observation::Present(_)
            ))
    }

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
                self.witnessed(&record, &node)?;
                self.commit(&Change::Remove(path.clone()))?;
                return Ok(Removal::Gone);
            }
            Observation::Present(live) => live,
        };
        if !self.ours(&live, path, Some(&record)) {
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
        let node = node_of(&record);
        match self.observe_node(&node, Some(&record))? {
            Observation::Absent => {
                self.witnessed(&record, &node)?;
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
