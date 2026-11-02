// SPDX-License-Identifier: Apache-2.0
//! The engine: a request names a stage and what to run in it, and the engine runs the Test
//! actions and validations that selects, in the order of the stage, under the run's lease.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use ucdk_ledger::{Change, DEFAULT_LEASE_TTL, LeaseGuard, Ledger, State};
use ucdk_release::codec;
use ucdk_release::machine::{Checked, tests, validations};
use ucdk_release::revision::{BuildStamp as CheckedStamp, Revision};
use ucdk_release::rows::{ExpectedRow, account_rows};
use ucdk_release::selection::{Holder, Item, Selection};
use ucdk_release_proto::v1::{
    Action, BuildStamp, BuildSystem as BuildSystemSpec, Reason, Row, Test, Validation, Verdict,
};
use ucdk_release_seams::{ArtifactStore, BuildSystem, Pause, Publisher};

use crate::leases::Leases;
use crate::outcome::{Finished, Flow, Outcome, Report};
use crate::records::{Records, row};

/// What the driver gives a run: who it is, and the facts every action of it sees. The engine
/// checks them before it takes the lease ([`Engine::handle`]): the attempt is at least 1, the
/// revision and the stamp's source commit are full commit ids, and the stamp's build number and
/// timestamp are positive. A run that breaks one is refused with `SOURCE`, and nothing is run or
/// written.
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
    Validation(&'s Holder, &'s Validation),
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
        if let Err(why) = self.check_run() {
            return Outcome::halted(Verdict::Refused, Reason::Source, why);
        }
        let named = machine
            .machine()
            .build_system
            .as_ref()
            .map(BuildSystemSpec::kind);
        let given = self.seams.build_system.kind();
        if named != Some(given) {
            return Outcome::halted(
                Verdict::Refused,
                Reason::BuildFailed,
                format!(
                    "the build system given is {}, and the machine names {}",
                    given.as_str_name(),
                    named.unwrap_or_default().as_str_name()
                ),
            );
        }
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
            // `resolve` names only what the stage declares, so a validation is always found;
            // an action is found only when it is a Test action.
            let (found, name) = match item {
                Item::Action { action, .. } => (
                    tests(stage)
                        .find(|(a, _)| a.name == *action)
                        .map(|(found, body)| Work::Test(found, body)),
                    action,
                ),
                Item::Validation { holder, name } => (
                    validations(stage)
                        .find(|v| v.name == *name)
                        .map(|decl| Work::Validation(holder, decl)),
                    name,
                ),
            };
            let Some(found) = found else {
                return Outcome::halted(
                    Verdict::Refused,
                    Reason::Selector,
                    format!(
                        "{name} is not a Test action; this engine runs Test actions and validations"
                    ),
                );
            };
            work.push(found);
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

    /// The facts of the run, checked by the rules of the checked revision and stamp types.
    fn check_run(&self) -> Result<(), String> {
        let run = &self.run;
        if run.attempt == 0 {
            return Err("the attempt is 0; attempts count from 1".to_owned());
        }
        Revision::parse(run.revision).map_err(|e| format!("the run's revision {e}"))?;
        let signed = |n: u64| i64::try_from(n).unwrap_or(i64::MAX);
        CheckedStamp::new(
            run.revision,
            &run.stamp.source_commit,
            signed(run.stamp.build_number),
            signed(run.stamp.timestamp_ms),
        )
        .map_err(|e| format!("the run's stamp: {e}"))?;
        Ok(())
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
