// SPDX-License-Identifier: Apache-2.0
//! The engine: observe, plan, check the approval, then carry the steps out.

use std::collections::BTreeMap;
use std::time::Duration;

use ucdk_ledger::{Change, Fence, LeaseError, Ledger, LedgerError, Record, State};
use ucdk_plan::{
    Action, LedgerToken, LiveState, Lowered, Observed, Plan, PlanHash, PlanNode, ROLE_RUN,
    Record as PlanRecord, Refused, Seen, Step, plan,
};
use ucdk_provider::{
    CellIdentity, Kind, Live, Node, NodePath, Observation, Provider, Stamp, StampEvidence,
    StateClass,
};

use crate::error::EngineError;
use crate::seam::{Failure, Mode, Outcome, Refusal, Report, Request};

/// How many times a ledger write that finds the document changed is read and tried again.
const WRITE_ATTEMPTS: usize = 5;

/// How long a run's lease lasts, and how long a run may spend on one step before it must renew:
/// the run renews at the start of every step, so a single step (its provider calls and the
/// ledger writes around them) has this long. A step that outlasts it ends the run at its next
/// ledger write, as a lost lease, and the next run resolves what it left.
pub const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(600);

/// The reconciler. It borrows the provider and the ledger it works on: one trait object each.
/// It works in one cell: every stamp it writes carries the cell's identity, and an object
/// that carries another cell's is not ours.
///
/// An apply or a destroy takes the ledger's lease for its whole run, writes under the lease's
/// fence, and gives the lease back when it ends. A plan or a status takes none.
pub struct Engine<'a> {
    pub(crate) provider: &'a mut dyn Provider,
    pub(crate) ledger: &'a dyn Ledger,
    pub(crate) cell: CellIdentity,
    /// Who the lease is taken as: what another run is told when it is refused.
    holder: String,
    ttl: Duration,
    /// The fence of the run in progress; [`Fence::NONE`] outside a run, which no write passes.
    fence: Fence,
}

/// The resource whose `run` node is at `path`, or none for any other role: a resource's
/// outputs are those of its `run` object.
fn run_resource(path: &NodePath) -> Option<&str> {
    path.as_str().strip_suffix(&format!("/{ROLE_RUN}"))
}

/// The node an existing record stands for, with no fields: enough to read the object by its
/// physical id, or by its name.
pub(crate) fn node_of(record: &Record) -> Node {
    Node::new(
        record.path.clone(),
        record.kind.clone(),
        record.name.clone(),
        Vec::new(),
    )
    .expect("a node with no fields has no repeated field")
}

impl<'a> Engine<'a> {
    pub fn new(provider: &'a mut dyn Provider, ledger: &'a dyn Ledger, cell: CellIdentity) -> Self {
        Self {
            provider,
            ledger,
            cell,
            holder: format!("ucdk-{}", std::process::id()),
            ttl: DEFAULT_LEASE_TTL,
            fence: Fence::NONE,
        }
    }

    /// The identity the run's lease is taken as. The default names the process.
    #[must_use]
    pub fn with_holder(mut self, holder: impl Into<String>) -> Self {
        self.holder = holder.into();
        self
    }

    /// How long the run's lease lasts before it is renewed (at every step).
    #[must_use]
    pub fn with_lease_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Whether the object carries the stamp of the node at `path` in this cell, and is the
    /// object the record names (when the record names one).
    pub(crate) fn ours(&self, live: &Live, path: &NodePath, record: Option<&Record>) -> bool {
        let stamped = match &live.stamp {
            StampEvidence::Present(stamp) => {
                stamp.same_identity(&Stamp::for_node(&self.cell, path))
            }
            _ => false,
        };
        let same = record
            .and_then(|r| r.physical.as_ref())
            .is_none_or(|physical| *physical == live.physical);
        stamped && same
    }

    /// The nodes of `view` whose kind the engine cannot apply yet (a stamp mode with no
    /// behaviour behind it).
    fn unapplicable(&self, view: &Lowered) -> Vec<NodePath> {
        view.nodes()
            .iter()
            .filter(|n| !self.provider.stamp_mode(n.node.kind()).applicable())
            .map(|n| n.node.path().clone())
            .collect()
    }

    /// The wanted nodes of `view` whose kind the provider does not host.
    fn unhosted(&self, view: &Lowered) -> Vec<(NodePath, Kind)> {
        view.nodes()
            .iter()
            .filter(|n| n.wanted && !self.provider.hosts(n.node.kind()))
            .map(|n| (n.node.path().clone(), n.node.kind().clone()))
            .collect()
    }

    /// The refusal for a view that wants a kind the provider does not host, if it does.
    pub(crate) fn refuse_unhosted(&self, view: &Lowered) -> Option<Outcome> {
        let unhosted = self.unhosted(view);
        (!unhosted.is_empty()).then_some(Outcome::Refused(Refusal::UnhostedKinds(unhosted)))
    }

    /// Handles one request.
    pub fn handle(&mut self, request: Request) -> Outcome {
        match request {
            Request::Plan { desired, mode } => match scope(&desired, mode) {
                Ok(view) => self.plan_of(&view),
                Err(refusal) => Outcome::Refused(refusal),
            },
            Request::Apply {
                desired,
                plan,
                plan_hash,
                rollback,
            } => match self.run(&desired, &plan, &plan_hash, rollback) {
                // The last step, after the lease is given back: a read that reports.
                Outcome::Done(mut report) => {
                    self.sweep_into(&mut report);
                    Outcome::Done(report)
                }
                other => other,
            },
            Request::Destroy {
                desired,
                plan,
                plan_hash,
            } => match scope(&desired, Mode::Destroy) {
                Ok(view) => self.run(&view, &plan, &plan_hash, false),
                Err(refusal) => Outcome::Refused(refusal),
            },
            Request::Status { desired } => self.status_of(&desired),
            Request::Leaks => self.leaks_of(),
        }
    }

    fn plan_of(&mut self, view: &Lowered) -> Outcome {
        if let Some(refused) = self.refuse_unhosted(view) {
            return refused;
        }
        let embedded = self.unapplicable(view);
        if !embedded.is_empty() {
            return Outcome::Refused(Refusal::UnsupportedStampMode(embedded));
        }
        match self.observe_all(view) {
            Err(error) => failed(Report::default(), None, error),
            Ok(observed) => match plan(view, &observed, None) {
                Ok(planned) => Outcome::Planned(planned),
                Err(Refused { conflicts }) => Outcome::Refused(Refusal::Conflicts(conflicts)),
            },
        }
    }

    fn run(&mut self, view: &Lowered, approved: &Plan, hash: &PlanHash, rollback: bool) -> Outcome {
        if let Some(refused) = self.refuse_unhosted(view) {
            return refused;
        }
        if approved.hash() != hash {
            return Outcome::Refused(Refusal::PlanHashMismatch {
                approved: hash.clone(),
                plan: approved.hash().clone(),
            });
        }
        let embedded = self.unapplicable(view);
        if !embedded.is_empty() {
            return Outcome::Refused(Refusal::UnsupportedStampMode(embedded));
        }
        let lease = match self.ledger.acquire(&self.holder, self.ttl) {
            Ok(lease) => lease,
            Err(LeaseError::Held { holder, remaining }) => {
                return Outcome::Refused(Refusal::LeaseHeld { holder, remaining });
            }
            Err(LeaseError::Unavailable(why)) => {
                // The grant may have landed with its answer lost: the lease may be ours.
                let error = EngineError::LeaseUnconfirmed { why, run: None };
                return failed(Report::default(), None, error);
            }
        };
        self.fence = lease.fence;
        let outcome = self.run_leased(view, hash, rollback);
        let released = self.ledger.release(lease.fence);
        self.fence = Fence::NONE;
        match (released, outcome) {
            // A finished run stands; the lease lapses on its own, or is taken again at once by
            // the same holder.
            (Err(LedgerError::Unavailable(why)), Outcome::Failed(mut failure)) => {
                failure.error = EngineError::LeaseUnconfirmed {
                    why,
                    run: Some(Box::new(failure.error)),
                };
                Outcome::Failed(failure)
            }
            (_, outcome) => outcome,
        }
    }

    /// The run itself, under the lease taken in [`Engine::run`].
    fn run_leased(&mut self, view: &Lowered, hash: &PlanHash, rollback: bool) -> Outcome {
        let observed = match self.observe_all(view) {
            Ok(observed) => observed,
            Err(error) => return failed(Report::default(), None, error),
        };
        let fresh = match plan(view, &observed, None) {
            Ok(fresh) => fresh,
            Err(Refused { conflicts }) => return Outcome::Refused(Refusal::Conflicts(conflicts)),
        };
        if fresh.hash() != hash {
            return Outcome::Refused(Refusal::PlanChanged {
                approved: hash.clone(),
                now: fresh.hash().clone(),
            });
        }
        let nodes: BTreeMap<&NodePath, &PlanNode> =
            view.nodes().iter().map(|n| (n.node.path(), n)).collect();
        let mut report = Report::default();
        for step in fresh.steps() {
            if let Err(error) = self.step(&nodes, step, &mut report) {
                // A run that lost its lease deletes nothing more: the new holder owns the cell.
                if rollback && error != EngineError::LeaseLost {
                    self.roll_back(&mut report);
                }
                return failed(report, Some(step.path.clone()), error);
            }
        }
        Outcome::Done(report)
    }

    fn step(
        &mut self,
        nodes: &BTreeMap<&NodePath, &PlanNode>,
        step: &Step,
        report: &mut Report,
    ) -> Result<(), EngineError> {
        self.ledger.renew(self.fence, self.ttl)?;
        match &step.action {
            // A node that reads an output is resolved here, in dependency order, after the
            // producer's step has confirmed its record and written the output.
            Action::Create | Action::Update | Action::KnownAfterApply => {
                self.converge(nodes[&step.path], report)
            }
            Action::Noop => self.settle(nodes[&step.path], report),
            Action::Converging => {
                report.converging.push(step.path.clone());
                Ok(())
            }
            Action::Delete { .. } => self.delete_step(&step.path, report),
            Action::Reap => self.reap_step(&step.path, report),
            Action::Retain(why) => {
                report.retained.push((step.path.clone(), why.clone()));
                Ok(())
            }
        }
    }

    /// Reads the ledger and every object the plan needs to know about: each node of the view,
    /// wanted or not, and each recorded path the view does not name.
    fn observe_all(&mut self, view: &Lowered) -> Result<Observed, EngineError> {
        let snapshot = self.ledger.read()?;
        let mut observed = Observed::new(
            LedgerToken::new(snapshot.token.to_string()),
            self.cell.clone(),
        );
        let mut subjects: BTreeMap<NodePath, Node> = snapshot
            .records
            .values()
            .map(|r| (r.path.clone(), node_of(r)))
            .collect();
        for n in view.nodes() {
            subjects.insert(n.node.path().clone(), n.node.clone());
        }
        for record in snapshot.records.values() {
            // Only a confirmed record's outputs are current; an intent's are the last known.
            let current = record.state == State::Confirmed;
            if let Some(resource) = run_resource(&record.path).filter(|_| current) {
                for (name, value) in &record.outputs {
                    observed = observed.with_output(resource, name, value);
                }
            }
        }
        for (path, node) in subjects {
            let record = snapshot.records.get(&path);
            let live = match self.observe_node(&node, record)? {
                Observation::Absent => None,
                Observation::Present(live) => Some(LiveState {
                    deletability: self.provider.deletability(node.kind(), &live),
                    live,
                }),
            };
            observed = observed.with(Seen {
                state_class: self.provider.state_class(node.kind()),
                kind: node.kind().clone(),
                live,
                record: record.map(|r| PlanRecord {
                    physical: r.physical.clone(),
                    confirmed: r.state != State::Intent,
                    deps: r.deps.clone(),
                }),
                path,
            });
        }
        Ok(observed)
    }

    /// Reads the object of `node`: by the physical id the record names, else by the node's name.
    pub(crate) fn observe_node(
        &mut self,
        node: &Node,
        record: Option<&Record>,
    ) -> Result<Observation, EngineError> {
        let hint = record.and_then(|r| r.physical.as_ref());
        Ok(self.provider.observe(node, hint)?)
    }

    pub(crate) fn record_of(&self, path: &NodePath) -> Result<Option<Record>, EngineError> {
        Ok(self.ledger.read()?.records.remove(path))
    }

    /// Writes `change`, reading the document again and trying again when another writer got in
    /// between: a change of one record does not depend on the others.
    pub(crate) fn commit(&self, change: &Change) -> Result<(), EngineError> {
        for _ in 0..WRITE_ATTEMPTS {
            let snapshot = self.ledger.read()?;
            match self
                .ledger
                .write_fenced(self.fence, &snapshot.token, change.clone())
            {
                Err(LedgerError::Stale) => {}
                other => return Ok(other.map(|_| ())?),
            }
        }
        Err(EngineError::LedgerBusy)
    }

    pub(crate) fn state_class(&self, kind: &Kind) -> StateClass {
        self.provider.state_class(kind)
    }
}

fn failed(report: Report, at: Option<NodePath>, error: EngineError) -> Outcome {
    Outcome::Failed(Failure { report, at, error })
}

/// The desired state a mode plans against: the file as it is, or none of it.
fn scope(desired: &Lowered, mode: Mode) -> Result<Lowered, Refusal> {
    match mode {
        Mode::Apply => Ok(desired.clone()),
        Mode::Destroy if desired.nodes().is_empty() => Err(Refusal::EmptyScope),
        Mode::Destroy => Ok(desired.unwanted()),
    }
}
