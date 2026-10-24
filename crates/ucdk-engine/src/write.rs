// SPDX-License-Identifier: Apache-2.0
//! The writing steps: create, update, and finding an object an earlier run left.

use ucdk_ledger::{Change, Record, State};
use ucdk_plan::PlanNode;
use ucdk_provider::{Live, Node, Observation, ProviderError, Refusal as CallRefusal, Stamp};

use crate::engine::{Engine, ours};
use crate::error::EngineError;
use crate::seam::Report;

/// A record for the node, as a first intent.
fn first_intent(pn: &PlanNode) -> Record {
    Record::intent(
        pn.node.path().clone(),
        pn.node.kind().clone(),
        pn.node.name(),
        pn.deps.clone(),
    )
}

/// The intent to write before a mutation: the record there is, back to an intent with its
/// edges and name as the file has them now, or a first intent.
fn intent_for(prior: Option<Record>, pn: &PlanNode) -> Record {
    let first = first_intent(pn);
    match prior {
        Some(record) => Record {
            state: State::Intent,
            name: first.name,
            deps: first.deps,
            ..record
        },
        None => first,
    }
}

impl Engine<'_> {
    /// The node as it will be applied. A value reference to another resource's output has no
    /// recorded value to fill in, so the node goes out with its literal fields.
    fn applied(pn: &PlanNode) -> Node {
        match pn.bound(|_, _| None) {
            Ok(node) => node,
            Err(_) => pn.node.clone(),
        }
    }

    /// A step the plan found nothing to do for. Only a record left as an intent needs a look:
    /// the object is there, so the record is confirmed.
    pub(crate) fn settle(&mut self, pn: &PlanNode, report: &mut Report) -> Result<(), EngineError> {
        match self.record_of(pn.node.path())? {
            Some(record) if record.state == State::Confirmed => Ok(()),
            _ => self.converge(pn, report),
        }
    }

    /// Makes the object of `pn` match the node: re-observe it, then create it, update it, or
    /// confirm it as it is.
    pub(crate) fn converge(
        &mut self,
        pn: &PlanNode,
        report: &mut Report,
    ) -> Result<(), EngineError> {
        let node = Self::applied(pn);
        let record = self.record_of(node.path())?;
        match self.observe_node(&node, record.as_ref())? {
            Observation::Absent => self.create(pn, &node, record, report),
            Observation::Present(live) => self.accept(pn, &node, record, &live, report),
        }
    }

    fn create(
        &mut self,
        pn: &PlanNode,
        node: &Node,
        prior: Option<Record>,
        report: &mut Report,
    ) -> Result<(), EngineError> {
        let intent = intent_for(prior, pn);
        self.commit(&Change::Put(intent.clone()))?;
        match self.provider.create(node, &Stamp::for_node(node.path())) {
            Ok(created) => {
                report.created.push(node.path().clone());
                // The record is confirmed only once a read shows the object. A read that
                // does not (it may lag behind the create) leaves the intent, now naming the
                // object, so that a later read of "gone" is never taken for proof.
                let named = Record {
                    physical: Some(created.physical.clone()),
                    ..intent.clone()
                };
                match self.observe_node(node, Some(&named)) {
                    Ok(Observation::Present(_)) => {
                        self.commit(&Change::Put(intent.confirmed(created.physical)))
                    }
                    Ok(Observation::Absent) => self.commit(&Change::Put(named)),
                    Err(error) => {
                        self.commit(&Change::Put(named))?;
                        Err(error)
                    }
                }
            }
            // Another client made the object between the read and the create. If it is ours
            // (our stamp), it is this node's object.
            Err(ProviderError::Refused(CallRefusal::AlreadyExists)) => {
                match self.observe_node(node, Some(&intent))? {
                    Observation::Present(live) if !ours(&live, node.path(), None) => {
                        // Not ours: this run created nothing, so the intent it wrote says
                        // nothing true. Left in the ledger, it would turn the foreign object
                        // into a stripped stamp on every later run.
                        self.withdraw(&intent)?;
                        Err(EngineError::NotOurs(node.path().clone()))
                    }
                    Observation::Present(live) => {
                        self.accept(pn, node, Some(intent), &live, report)
                    }
                    Observation::Absent => {
                        Err(ProviderError::Refused(CallRefusal::AlreadyExists).into())
                    }
                }
            }
            // Rejected before any effect: the intent says nothing that is true, so it goes.
            Err(error @ ProviderError::Refused(_)) => {
                self.commit(&Change::Remove(node.path().clone()))?;
                Err(error.into())
            }
            // Anything else may have changed something: the intent stays for the next run.
            Err(error) => Err(error.into()),
        }
    }

    /// Removes the record of `intent`, the intent this run wrote, if the record is still
    /// exactly that: a record another writer has changed since is theirs and stays.
    fn withdraw(&self, intent: &Record) -> Result<(), EngineError> {
        if self.record_of(&intent.path)?.as_ref() == Some(intent) {
            self.commit(&Change::Remove(intent.path.clone()))?;
        }
        Ok(())
    }

    /// The object is there. If it is ours, bring it to the node and confirm the record;
    /// if it is not, the run stops.
    fn accept(
        &mut self,
        pn: &PlanNode,
        node: &Node,
        record: Option<Record>,
        live: &Live,
        report: &mut Report,
    ) -> Result<(), EngineError> {
        if !ours(live, node.path(), record.as_ref()) {
            return Err(EngineError::NotOurs(node.path().clone()));
        }
        let was_confirmed = record.as_ref().is_some_and(|r| r.state == State::Confirmed);
        let base = if live.digest == node.digest() {
            if was_confirmed {
                return Ok(());
            }
            report.resumed.push(node.path().clone());
            intent_for(record, pn)
        } else {
            // Written before the update, with the object's id: a crash leaves a record that
            // names the object the update may have changed.
            let intent = Record {
                physical: Some(live.physical.clone()),
                ..intent_for(record, pn)
            };
            self.commit(&Change::Put(intent.clone()))?;
            self.provider.update(node, live)?;
            report.updated.push(node.path().clone());
            intent
        };
        self.commit(&Change::Put(base.confirmed(live.physical.clone())))
    }
}
