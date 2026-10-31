// SPDX-License-Identifier: Apache-2.0
//! A Test action: the build system, run once per target with the layered flags of the machine,
//! one derived row per target. A target passes only when the build system exits 0 and its
//! report shows at least one test that ran. The action stops at the first target that is not a
//! pass, and the later targets are not reached.
//!
//! The record of the action is written as an intent before the first invocation and as
//! confirmed after the last pass. A target that failed or ran nothing leaves the intent, which
//! a rerun runs again: a failure is not a state, and the report of the run that failed names
//! the target.

use std::fs;
use std::path::Path;

use ucdk_ledger::State;
use ucdk_release::action_input::{Given, action_input};
use ucdk_release::codec;
use ucdk_release::hashes::machine_hash;
use ucdk_release::invocation::report_file_name;
use ucdk_release::machine::stage_platform;
use ucdk_release::rows::ExpectedRow;
use ucdk_release_proto::v1::{Action, Reason, Row, RowState, Stage, Test, Verdict};
use ucdk_release_seams::{Invocation, SeamError, Verb};

use crate::engine::{Engine, accounted};
use crate::leases::Leases;
use crate::outcome::{Finished, Flow, Report, action_report, row, what_failed};
use crate::records::Records;

/// What every row of a Test action asserts of its target.
pub const TEST_ASSERTION: &str = "the test target passes";

/// What one invocation of the build system came to.
enum Seen {
    /// The lease was lost during the invocation, and the runner stopped what it started.
    Lost,
    Row(RowState, String),
}

impl Engine<'_> {
    pub(crate) fn test_action(
        &mut self,
        leases: &Leases<'_>,
        stage: &Stage,
        action: &Action,
        body: &Test,
    ) -> Flow<Finished> {
        let given = Given {
            revision: self.run.revision,
            stamp: self.run.stamp,
            set: None,
        };
        let input = action_input(self.machine, &stage.name, &action.name, &given)
            .expect("a Test action of a checked machine has an input and takes no set");
        let records = self.records()?;
        match records.action(&stage.name, &action.name) {
            Some((found, State::Confirmed)) if found.input == input => {
                let message = "confirmed already, with the same inputs";
                return Ok(Finished {
                    verdict: Verdict::Noop,
                    reason: Reason::Unspecified,
                    message: String::new(),
                    report: Some(Box::new(Report::Action(action_report(
                        &action.name,
                        Verdict::Noop,
                        Reason::Unspecified,
                        message,
                        Vec::new(),
                    )))),
                });
            }
            Some((_, State::Confirmed)) => {
                return Err(Finished::stopped(
                    Verdict::Refused,
                    Reason::InputsChanged,
                    format!(
                        "the action {} of the stage {} was confirmed for other inputs; \
                         use a new run id",
                        action.name, stage.name
                    ),
                ));
            }
            _ => {}
        }
        self.pin(leases, &records)?;
        let record = codec::Record::Action(codec::Action {
            stage: stage.name.clone(),
            action: action.name.clone(),
            input,
            set: None,
            generation: None,
        });
        self.put(leases, &record, State::Intent)?;

        let rows = self.targets(leases, stage, action, body)?;
        let expected: Vec<ExpectedRow> = body
            .targets
            .iter()
            .map(|t| ExpectedRow::new(&t.label, TEST_ASSERTION))
            .collect();
        let verdict = accounted(&expected, &rows)?;
        let (reason, message) = match verdict {
            Verdict::Succeeded => (Reason::Unspecified, String::new()),
            Verdict::Indeterminate => (Reason::BuildFailed, what_failed(&rows)),
            _ => (Reason::Validation, what_failed(&rows)),
        };
        if verdict == Verdict::Succeeded {
            leases.settle()?;
            self.put(leases, &record, State::Confirmed)?;
        }
        Ok(Finished {
            verdict,
            reason,
            message: message.clone(),
            report: Some(Box::new(Report::Action(action_report(
                &action.name,
                verdict,
                reason,
                &message,
                rows,
            )))),
        })
    }

    /// Checks the run record against the run this engine was given, and writes it when the
    /// ledger has none: the machine, revision and stamp are pinned by the first stage that
    /// holds a Build or Test action, and a later request that presents another is refused.
    fn pin(&self, leases: &Leases<'_>, records: &Records) -> Flow<()> {
        let stamp = self.run.stamp;
        let wanted = codec::Run {
            run_id: self.run.id.to_owned(),
            machine: machine_hash(self.machine.machine()),
            revision: self.run.revision.to_owned(),
            version: stamp.version.clone(),
            build: stamp.build_number,
            commit: stamp.source_commit.clone(),
            timestamp: stamp.timestamp_ms,
        };
        let record = codec::Record::Run(wanted.clone());
        let Some((found, state)) = records.run() else {
            self.put(leases, &record, State::Intent)?;
            return self.put(leases, &record, State::Confirmed);
        };
        let differs: Vec<&str> = [
            ("run id", found.run_id != wanted.run_id),
            ("machine", found.machine != wanted.machine),
            ("revision", found.revision != wanted.revision),
            ("release version", found.version != wanted.version),
            ("build number", found.build != wanted.build),
            ("source commit", found.commit != wanted.commit),
            ("timestamp", found.timestamp != wanted.timestamp),
        ]
        .into_iter()
        .filter(|(_, differs)| *differs)
        .map(|(name, _)| name)
        .collect();
        if !differs.is_empty() {
            return Err(Finished::stopped(
                Verdict::Refused,
                Reason::PinMismatch,
                format!(
                    "the run record pins other values for: {}; use the machine, revision and \
                     stamp the run began with",
                    differs.join(", ")
                ),
            ));
        }
        // A run that was killed between the two writes left an intent for the same values.
        if state == State::Confirmed {
            Ok(())
        } else {
            self.put(leases, &record, State::Confirmed)
        }
    }

    /// Runs each target in the order listed, stopping at the first that is not a pass; the rest
    /// are rows that were not reached.
    fn targets(
        &mut self,
        leases: &Leases<'_>,
        stage: &Stage,
        action: &Action,
        body: &Test,
    ) -> Flow<Vec<Row>> {
        let system = self
            .machine
            .machine()
            .build_system
            .clone()
            .unwrap_or_default();
        let report_name = report_file_name(self.seams.build_system.kind()).unwrap_or_default();
        let platform = stage_platform(stage).unwrap_or_default();
        let mut rows = Vec::new();
        let mut reached = true;
        for (index, target) in body.targets.iter().enumerate() {
            let name = ExpectedRow::new(&target.label, TEST_ASSERTION).name();
            if !reached {
                let detail = "an earlier target did not pass";
                rows.push(row(&name, RowState::RowNotReached, detail));
                continue;
            }
            leases.renew()?;
            let dir = self.scratch.join(format!(
                "{}-{}-a{}-{index}",
                stage.name, action.name, self.run.attempt
            ));
            let report = dir.join(report_name).display().to_string();
            let invocation = Invocation {
                system: &system,
                target,
                verb: Verb::Test,
                report: Some(&report),
                stamp: self.run.stamp,
                revision: self.run.revision,
                platform,
            };
            match self.invoke(leases, &dir, &invocation) {
                Seen::Lost => {
                    return Err(Finished::stopped(
                        Verdict::Interrupted,
                        Reason::LeaseLost,
                        format!(
                            "the lease was lost while {} ran; nothing more was recorded",
                            target.label
                        ),
                    ));
                }
                Seen::Row(state, detail) => {
                    reached = state == RowState::RowPassed;
                    rows.push(row(&name, state, detail));
                }
            }
        }
        Ok(rows)
    }

    /// One invocation of the build system in a report directory of its own, made for it and
    /// removed after it. A result is a pass only when the build system says so and a test ran.
    fn invoke(&mut self, leases: &Leases<'_>, dir: &Path, invocation: &Invocation<'_>) -> Seen {
        let _ = fs::remove_dir_all(dir);
        if let Err(e) = fs::create_dir_all(dir) {
            return Seen::Row(
                RowState::RowUnreadable,
                format!(
                    "the report directory {} could not be made: {e}",
                    dir.display()
                ),
            );
        }
        let seen = match self.seams.build_system.test(invocation, leases) {
            Ok(done) if done.passed && done.ran > 0 => Seen::Row(RowState::RowPassed, done.tail),
            Ok(done) => Seen::Row(
                RowState::RowFailed,
                if done.ran == 0 {
                    "no test ran".to_owned()
                } else {
                    done.tail
                },
            ),
            Err(SeamError::Refused(Reason::LeaseLost)) => Seen::Lost,
            Err(SeamError::Permanent(why)) => Seen::Row(RowState::RowFailed, why),
            Err(other) => Seen::Row(RowState::RowUnreadable, other.to_string()),
        };
        let _ = fs::remove_dir_all(dir);
        seen
    }
}
