// SPDX-License-Identifier: Apache-2.0
//! The engine: a request names a stage and what to run in it, and the engine runs the Test
//! actions and validations that selects, in the order of the stage, under the run's lease.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use ucdk_ledger::{Change, DEFAULT_LEASE_TTL, LeaseGuard, Ledger, State};
use ucdk_release::codec;
use ucdk_release::machine::{Checked, tests, validations};
use ucdk_release::rows::{ExpectedRow, account_rows};
use ucdk_release::selection::{Holder, Item, Selection};
use ucdk_release_proto::v1::{Action, BuildStamp, Reason, Row, Test, Validation, Verdict};
use ucdk_release_seams::{ArtifactStore, BuildSystem, Pause, Publisher};

use crate::leases::Leases;
use crate::outcome::{Finished, Flow, Outcome, Report};
use crate::records::{Records, row};

/// What the driver gives a run: who it is, and the facts every action of it sees.
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    /// The run id. One release is one run id across all its stages.
    pub id: &'a str,
    /// The attempt, from 1; a rerun of a stage that did not finish raises it.
    pub attempt: u32,
    /// The commit the run is about.
    pub revision: &'a str,
    /// The build stamp of the run, which the build system reads through placeholders.
    pub stamp: &'a BuildStamp,
}

/// The seams the engine works through. The engine holds no other way to the world: it makes no
/// call of its own to a build system, a store or a registry.
pub struct Seams<'a> {
    pub build_system: &'a mut dyn BuildSystem,
    pub store: &'a mut dyn ArtifactStore,
    /// The publisher of each channel the machine declares, by channel name.
    pub channels: BTreeMap<String, &'a mut dyn Publisher>,
    pub pause: &'a mut dyn Pause,
}

/// What to run: a stage, and the `--only` selectors, none meaning all of the stage.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub stage: String,
    pub only: Selection,
}

/// The engine of one run.
pub struct Engine<'a> {
    pub(crate) machine: &'a Checked,
    pub(crate) ledger: &'a dyn Ledger,
    pub(crate) run: Run<'a>,
    pub(crate) seams: Seams<'a>,
    pub(crate) scratch: &'a Path,
    ttl: Duration,
}

/// One item of a request, resolved against the stage.
enum Work<'s> {
    Test(&'s Action, &'s Test),
    Validation(&'s Holder, Validation),
}

impl<'a> Engine<'a> {
    /// An engine for `run` of `machine`, writing its records to `ledger` and making the
    /// directory of each test invocation's report under `scratch`.
    pub fn new(
        machine: &'a Checked,
        ledger: &'a dyn Ledger,
        run: Run<'a>,
        seams: Seams<'a>,
        scratch: &'a Path,
    ) -> Self {
        Self {
            machine,
            ledger,
            run,
            seams,
            scratch,
            ttl: DEFAULT_LEASE_TTL,
        }
    }

    /// The lease lasts `ttl`, renewed on progress and before each effect.
    #[must_use]
    pub fn with_lease_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Runs what `request` selects, in the order of the stage, and stops at the first item that
    /// is not `Succeeded` or `Noop`. Everything that needs no ledger is checked first, so a
    /// request that is refused for its shape takes no lease and writes nothing.
    pub fn handle(&mut self, request: &Request) -> Outcome {
        let machine = self.machine;
        let Some(stage) = machine.stage(&request.stage) else {
            return Outcome::halted(
                Verdict::Refused,
                Reason::StageUnknown,
                format!("the machine has no stage {}", request.stage),
            );
        };
        let items = match request.only.resolve(stage) {
            Ok(items) => items,
            Err(e) => return Outcome::halted(Verdict::Refused, e.reason(), e.to_string()),
        };
        let mut work = Vec::new();
        for item in &items {
            match item {
                Item::Action { action, .. } => {
                    match tests(stage).find(|(a, _)| a.name == *action) {
                        Some((found, body)) => work.push(Work::Test(found, body)),
                        None => {
                            return Outcome::halted(
                                Verdict::Refused,
                                Reason::Selector,
                                format!(
                                    "{action} is not a Test action; this engine runs Test actions \
                                 and validations"
                                ),
                            );
                        }
                    }
                }
                Item::Validation { holder, name } => {
                    let decl = validations(stage)
                        .find(|v| v.name == *name)
                        .cloned()
                        .unwrap_or_default();
                    work.push(Work::Validation(holder, decl));
                }
            }
        }
        let holder = format!("run:{}/{}", self.run.id, stage.name);
        let guard = match LeaseGuard::acquire(self.ledger, holder, self.ttl) {
            Ok(guard) => guard,
            Err(e) => {
                let stopped = Finished::from(e);
                return Outcome::halted(stopped.verdict, stopped.reason, stopped.message);
            }
        };
        let leases = Leases::new(guard);
        let mut outcome = Outcome::halted(Verdict::Noop, Reason::Unspecified, "");
        for one in &work {
            let (Ok(done) | Err(done)) = match one {
                Work::Test(action, body) => self.test_action(&leases, stage, action, body),
                Work::Validation(holder, decl) => self.validation(&leases, stage, holder, decl),
            };
            let goes_on = done.goes_on();
            let passed = done.verdict == Verdict::Succeeded;
            if let Some(report) = done.report {
                match *report {
                    Report::Action(report) => outcome.actions.push(report),
                    Report::Validation(report) => outcome.validations.push(report),
                }
            }
            if !goes_on {
                outcome.verdict = done.verdict;
                outcome.reason = done.reason;
                outcome.message = done.message;
                break;
            }
            // An item that was skipped does not hide one that ran.
            if passed {
                outcome.verdict = Verdict::Succeeded;
            }
        }
        leases.release();
        outcome
    }

    /// The release records of the run ledger.
    pub(crate) fn records(&self) -> Flow<Records> {
        let snapshot = self.ledger.read()?;
        Ok(Records::decode(&snapshot)?)
    }

    /// Writes `record` in `state` under the lease.
    pub(crate) fn put(
        &self,
        leases: &Leases<'_>,
        record: &codec::Record,
        state: State,
    ) -> Flow<()> {
        let written = row(record, state)?;
        Ok(leases.commit(&Change::Put(written))?)
    }
}

/// The verdict the evaluated `rows` come to against the `expected` ones.
pub(crate) fn accounted(expected: &[ExpectedRow], rows: &[Row]) -> Flow<Verdict> {
    Ok(account_rows(expected, rows)?)
}
