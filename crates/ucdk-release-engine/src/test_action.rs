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
//!
//! Each invocation writes its report into a directory of its own under the scratch directory,
//! allocated with an exclusive create so that no other invocation, of this engine or of another
//! one sharing the scratch directory, can hold the same one, and a directory left behind by an
//! earlier invocation is never reused. It is removed after the invocation; a directory that
//! cannot be removed makes the row unreadable, never a pass.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use ucdk_ledger::{GuardError, State};
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
    /// A progress event could not keep the lease, and the runner stopped what it started: the
    /// lease was lost, or the ledger could not be read or written.
    Stopped(GuardError),
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
        let input = action_input(self.machine, &stage.name, &action.name, &given)?;
        let records = self.records()?;
        match records.action(&stage.name, &action.name) {
            Some((found, State::Confirmed)) if found.input == input => {
                // The input digest leaves out the run id, the timestamp and the parts of the
                // machine the action does not read: a result is taken as this run's only when
                // the run record pins this run.
                self.check_pin(&records)?;
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
        // The rows were made from the same targets as the derived ones; a fault the accounting
        // finds all the same stops the request as a spec fault, never a pass.
        let verdict = accounted(&expected, &rows)?;
        let (reason, message) = match verdict {
            Verdict::Succeeded => (Reason::Unspecified, String::new()),
            Verdict::Indeterminate => (Reason::BuildFailed, what_failed(&rows)),
            _ => (Reason::Validation, what_failed(&rows)),
        };
        // Whatever the work came to, a lease another holder took while it ran is what the
        // request reports.
        leases.settle()?;
        if verdict == Verdict::Succeeded {
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
        let record = codec::Record::Run(self.wanted_run());
        match self.check_pin(records)? {
            None => {
                self.put(leases, &record, State::Intent)?;
                self.put(leases, &record, State::Confirmed)
            }
            Some(State::Confirmed) => Ok(()),
            // A run that was killed between the two writes left an intent for the same values.
            Some(_) => self.put(leases, &record, State::Confirmed),
        }
    }

    /// The run record this engine's run pins.
    fn wanted_run(&self) -> codec::Run {
        let stamp = self.run.stamp;
        codec::Run {
            run_id: self.run.id.to_owned(),
            machine: machine_hash(self.machine.machine()),
            revision: self.run.revision.to_owned(),
            version: stamp.version.clone(),
            build: stamp.build_number,
            commit: stamp.source_commit.clone(),
            timestamp: stamp.timestamp_ms,
        }
    }

    /// Refuses a run record that pins other values than this run's, naming them, and says in
    /// what state the record is, or that there is none. Writes nothing.
    pub(crate) fn check_pin(&self, records: &Records) -> Flow<Option<State>> {
        let wanted = self.wanted_run();
        let Some((found, state)) = records.run() else {
            return Ok(None);
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
        Ok(Some(state))
    }

    /// Runs each target in the order listed, stopping at the first that is not a pass; the rest
    /// are rows that were not reached. Before each target the lease is made good again
    /// ([`Leases::settle`]), so a target that ran silently past the lease does not stop the
    /// next one when nothing else touched the ledger, and a later target never starts under a
    /// lease another holder took.
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
            leases.settle()?;
            let dir = match self.allocate(&stage.name, &action.name, index) {
                Ok(dir) => dir,
                Err(e) => {
                    let detail = format!(
                        "a report directory could not be made under {}: {e}",
                        self.scratch.display()
                    );
                    reached = false;
                    rows.push(row(&name, RowState::RowUnreadable, detail));
                    continue;
                }
            };
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
                Seen::Stopped(error) => {
                    let mut stop = Finished::from(error);
                    stop.message = format!(
                        "{}; the invocation of {} was stopped and nothing more was recorded",
                        stop.message, target.label
                    );
                    return Err(stop);
                }
                Seen::Row(state, detail) => {
                    reached = state == RowState::RowPassed;
                    rows.push(row(&name, state, detail));
                }
            }
        }
        Ok(rows)
    }

    /// A report directory for the invocation of target `index` of `stage`/`action`, made with an
    /// exclusive create: the name is a digest of the run, the attempt and the target, and a
    /// counter that moves past every name another invocation or an earlier one holds.
    fn allocate(&self, stage: &str, action: &str, index: usize) -> io::Result<PathBuf> {
        let mut hasher = Sha256::new();
        for part in [
            self.run.id,
            stage,
            action,
            &self.run.attempt.to_string(),
            &index.to_string(),
        ] {
            // Each part is preceded by its length, so no two lists of parts read the same.
            hasher.update((part.len() as u64).to_be_bytes());
            hasher.update(part.as_bytes());
        }
        let digest = hasher.finalize();
        let key: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        let mut n: u64 = 0;
        loop {
            let dir = self.scratch.join(format!("test-{key}-{n}"));
            match fs::create_dir(&dir) {
                Ok(()) => return Ok(dir),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e),
            }
        }
    }

    /// One invocation of the build system in the report directory allocated for it, removed
    /// after it. A result is a pass only when the build system says so and a test ran.
    fn invoke(&mut self, leases: &Leases<'_>, dir: &Path, invocation: &Invocation<'_>) -> Seen {
        let result = self.seams.build_system.test(invocation, leases);
        let seen = match (leases.beat_failure(), result) {
            (Some(error), _) => Seen::Stopped(error),
            (None, Err(SeamError::Refused(Reason::LeaseLost))) => Seen::Stopped(GuardError::Lost),
            (None, Ok(done)) if done.passed && done.ran > 0 => {
                Seen::Row(RowState::RowPassed, done.tail)
            }
            (None, Ok(done)) => Seen::Row(
                RowState::RowFailed,
                if done.ran == 0 {
                    "no test ran".to_owned()
                } else {
                    done.tail
                },
            ),
            (None, Err(SeamError::Permanent(why))) => Seen::Row(RowState::RowFailed, why),
            (None, Err(other)) => Seen::Row(RowState::RowUnreadable, other.to_string()),
        };
        // A stop stands whatever the removal came to; a row does not, when the directory stays.
        match (seen, fs::remove_dir_all(dir)) {
            (Seen::Row(..), Err(e)) => Seen::Row(
                RowState::RowUnreadable,
                format!(
                    "the report directory {} could not be removed: {e}",
                    dir.display()
                ),
            ),
            (seen, _) => seen,
        }
    }
}
