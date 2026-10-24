// SPDX-License-Identifier: Apache-2.0
//! The writing steps: create, update, and finding an object an earlier run left.

use std::collections::BTreeMap;

use ucdk_ledger::{Change, Record, State};
use ucdk_plan::{PlanNode, ROLE_RUN};
use ucdk_provider::{
    Live, Node, NodePath, Observation, ProviderError, Refusal as CallRefusal, Stamp, Status,
};

use crate::engine::Engine;
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
    /// The node as it will be applied: its literal fields plus each output it reads, as the
    /// ledger records it for the producer now. `None` when a producer is not confirmed yet (it
    /// is still coming up), so the value does not exist and the node waits for a later run. A
    /// producer that is confirmed and has no such output is an error.
    fn applied(&self, pn: &PlanNode) -> Result<Option<Node>, EngineError> {
        if pn.refs.is_empty() {
            return Ok(Some(pn.node.clone()));
        }
        let records = self.ledger.read()?.records;
        let producer = |resource: &str| {
            let path = NodePath::new(&format!("{resource}/{ROLE_RUN}"))
                .expect("a resource id is one valid segment");
            records.get(&path)
        };
        let bound = pn.bound(|resource, output| {
            producer(resource)
                .and_then(|record| record.outputs.get(output))
                .map(String::as_str)
        });
        match bound {
            Ok(node) => Ok(Some(node)),
            Err(pending) => match pending
                .iter()
                .find(|r| producer(&r.resource).is_some_and(|p| p.state == State::Confirmed))
            {
                Some(r) => Err(EngineError::OutputUnknown {
                    path: pn.node.path().clone(),
                    resource: r.resource.clone(),
                    output: r.output.as_str_name().to_owned(),
                }),
                None => Ok(None),
            },
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
        let Some(node) = self.applied(pn)? else {
            report.converging.push(pn.node.path().clone());
            return Ok(());
        };
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
        // A create is made because the object was read as absent, so an id the record held
        // names an object that is gone. The intent carries none: the new id is written once
        // the create returns it.
        let intent = Record {
            physical: None,
            outputs: BTreeMap::new(),
            ..intent_for(prior, pn)
        };
        self.commit(&Change::Put(intent.clone()))?;
        let stamp = Stamp::for_node(&self.cell, node.path());
        match self.provider.create(node, &stamp) {
            Ok(created) => {
                report.created.push(node.path().clone());
                // The record is confirmed only once a read shows the object, and shows it
                // ready. A read that does not (it may lag behind the create, or the object may
                // still be coming up) leaves the intent, now naming the object, so that a
                // later read of "gone" is never taken for proof.
                let named = Record {
                    physical: Some(created.physical.clone()),
                    ..intent.clone()
                };
                match self.observe_node(node, Some(&named)) {
                    Ok(Observation::Present(live)) if live.status == Status::Ready => {
                        let confirmed = intent.confirmed(created.physical);
                        self.commit(&Change::Put(confirmed.with_outputs(&live.outputs)))
                    }
                    Ok(Observation::Present(live)) => {
                        note_not_ready(report, node, live.status);
                        self.commit(&Change::Put(named))
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
                    Observation::Present(live) if !self.ours(&live, node.path(), None) => {
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
    /// if it is not, the run stops. An object that is still converging is left as it is, with
    /// its record unconfirmed; a failed one is updated even when its fields match.
    fn accept(
        &mut self,
        pn: &PlanNode,
        node: &Node,
        record: Option<Record>,
        live: &Live,
        report: &mut Report,
    ) -> Result<(), EngineError> {
        if !self.ours(live, node.path(), record.as_ref()) {
            return Err(EngineError::NotOurs(node.path().clone()));
        }
        if live.status == Status::Converging {
            report.converging.push(node.path().clone());
            return Ok(());
        }
        let was_confirmed = record.as_ref().is_some_and(|r| r.state == State::Confirmed);
        let (base, outputs) = if live.digest == node.digest() && live.status == Status::Ready {
            if was_confirmed {
                return Ok(());
            }
            report.resumed.push(node.path().clone());
            (intent_for(record, pn), live.outputs.clone())
        } else {
            // Written before the update, with the object's id: a crash leaves a record that
            // names the object the update may have changed.
            let intent = Record {
                physical: Some(live.physical.clone()),
                ..intent_for(record, pn)
            };
            self.commit(&Change::Put(intent.clone()))?;
            let updated = self.provider.update(node, live)?;
            report.updated.push(node.path().clone());
            if live.status == Status::Failed {
                return self.confirm_when_ready(node, intent, report);
            }
            (intent, updated.outputs)
        };
        let confirmed = base.confirmed(live.physical.clone());
        self.commit(&Change::Put(confirmed.with_outputs(&outputs)))
    }

    /// After the update of a failed object: read it again and confirm the record only if the
    /// object is ready. One that is still coming up, or failed again, or not yet visible
    /// leaves the intent, which names the object, for the next run.
    fn confirm_when_ready(
        &mut self,
        node: &Node,
        intent: Record,
        report: &mut Report,
    ) -> Result<(), EngineError> {
        match self.observe_node(node, Some(&intent))? {
            Observation::Present(now) if now.status == Status::Ready => {
                let confirmed = intent.confirmed(now.physical);
                self.commit(&Change::Put(confirmed.with_outputs(&now.outputs)))
            }
            Observation::Present(now) => {
                note_not_ready(report, node, now.status);
                Ok(())
            }
            Observation::Absent => Ok(()),
        }
    }
}

/// Records, in the report, an object that read as not ready after this run wrote to it.
fn note_not_ready(report: &mut Report, node: &Node, status: Status) {
    match status {
        Status::Ready => {}
        Status::Converging => report.converging.push(node.path().clone()),
        Status::Failed => report.failed.push(node.path().clone()),
    }
}
