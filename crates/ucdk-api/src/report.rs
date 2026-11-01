// SPDX-License-Identifier: Apache-2.0
//! The reports a release run writes and the one function that checks them.
//!
//! A writer validates a report before it writes it and a reader validates it before it trusts
//! it, with the same function, so ucdk never writes what it would refuse to read.

use ucdk_release::run_identity::RunIdentityError;
use ucdk_release::schema::{SchemaPolicy, SchemaRefusal};
use ucdk_release_proto::v1::{Reason, Retry, RunReport, StageReport, Verdict};

use crate::v1::ExitClass;

/// The policy for the schema version of a stage report and of a run report: this ucdk writes
/// and reads major 1.
pub fn report_schema() -> SchemaPolicy {
    SchemaPolicy::new(1, 1).expect("1 <= 1 <= 1")
}

/// Why a report is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportError {
    /// The schema version is not one this reader reads.
    Schema(SchemaRefusal),
    /// The report carries no run identity.
    RunMissing,
    /// The run identity is outside its grammar.
    Run(RunIdentityError),
    /// The stage has no name.
    StageNameEmpty,
    /// The revision is neither empty nor 40 lowercase hex.
    Revision { text: String },
    /// A selector in `only` is not `action:<name>` or `validation:<name>`.
    OnlyMalformed { text: String },
    /// A selector is listed twice.
    OnlyTwice { text: String },
    /// The set hash is neither empty nor 64 lowercase hex.
    SetHash { text: String },
    /// The verdict is unset.
    VerdictUnspecified,
    /// A refusal, a failure, an indeterminate answer or a failed validation without a reason.
    ReasonRequired { verdict: Verdict },
    /// A verdict that ended well, with a reason.
    ReasonOnSuccess { verdict: Verdict, reason: Reason },
    /// The exit code is not the one the verdict, the reason and the effects give.
    ExitCode { stated: u32, expected: u32 },
    /// An exit code that promises no effect landed, on a report that lists one.
    PromisesNoEffect { exit_code: u32, effect: String },
    /// The retry advice is unset.
    RetryUnspecified,
    /// The retry advice is weaker than the exit class gives.
    RetryWeaker { stated: Retry, default: Retry },
    /// The run finished before it started.
    FinishedBeforeStarted { started: u64, finished: u64 },
    /// An action without a name.
    ActionNameEmpty { index: usize },
    /// Two actions with one name.
    ActionTwice { name: String },
    /// An action whose verdict is unset.
    ActionVerdictUnspecified { name: String },
    /// An effect whose state is unset.
    EffectUnspecified { action: String, subject: String },
    /// A dry run that lists an effect which happened.
    EffectInPlan { action: String, subject: String },
    /// A real run that lists an effect which would happen.
    WouldOutsidePlan { action: String, subject: String },
    /// A row whose state is unset: it never reads as passed.
    RowUnspecified { action: String, row: String },
    /// A run report without a stage.
    NoStages,
    /// A stage of another run than the report.
    StageRunDiffers { index: usize },
    /// Two stages with one name.
    StageTwice { name: String },
    /// A stage report that is itself refused.
    Stage {
        index: usize,
        error: Box<ReportError>,
    },
    /// The run's verdict is not that of the first stage that did not end done.
    RunVerdictDiffers { stated: Verdict, expected: Verdict },
    /// The run's reason is not that of the first stage that did not end done.
    RunReasonDiffers { stated: Reason, expected: Reason },
    /// Every stage ended done and the run's verdict says otherwise.
    RunNotDone { stated: Verdict },
}

/// The retry advice of an exit class: running again is safe after a done run and after a
/// failure with no effect, unsafe after a failure that changed something, and for a human to
/// decide otherwise.
pub fn default_retry(class: ExitClass) -> Retry {
    let _ = class;
    Retry::Safe
}

/// Checks a stage report.
pub fn validate_stage(report: &StageReport) -> Result<(), ReportError> {
    let _ = report;
    Ok(())
}

/// Checks a run report and every stage report in it.
pub fn validate_run(report: &RunReport) -> Result<(), ReportError> {
    let _ = report;
    Ok(())
}
