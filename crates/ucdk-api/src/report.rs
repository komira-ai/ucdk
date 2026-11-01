// SPDX-License-Identifier: Apache-2.0
//! The reports a release run writes and the one function that checks them.
//!
//! A writer validates a report before it writes it and a reader validates it before it trusts
//! it, with the same function, so ucdk never writes what it would refuse to read.

use std::collections::BTreeSet;
use std::fmt;

use ucdk_release::run_identity::{self, RunIdentityError};
use ucdk_release::schema::{SchemaPolicy, SchemaRefusal};
use ucdk_release::selection::Selection;
use ucdk_release_proto::v1::{
    ActionReport, EffectState, Reason, Retry, RowState, RunIdentity, RunReport, StageReport,
    Verdict,
};

use crate::exit_class;
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
    /// The verdict is unset or not one this reader knows.
    VerdictUnspecified,
    /// The reason is not one this reader knows.
    ReasonUnknown { number: i32 },
    /// A refusal, a failure, an indeterminate answer or a failed validation without a reason.
    ReasonRequired { verdict: Verdict },
    /// A verdict that ended well, with a reason.
    ReasonOnSuccess { verdict: Verdict, reason: Reason },
    /// The exit code is not the one the verdict, the reason and the effects give.
    ExitCode { stated: u32, expected: u32 },
    /// An exit code that promises no effect landed, on a report that lists one.
    PromisesNoEffect { exit_code: u32, effect: String },
    /// The retry advice is unset or not one this reader knows.
    RetryUnspecified,
    /// The retry advice is weaker than the exit class gives.
    RetryWeaker { stated: Retry, default: Retry },
    /// The stage finished before it started (both times known).
    FinishedBeforeStarted {
        stage: String,
        started: u64,
        finished: u64,
    },
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
    /// A stage that did not end done is followed by another stage: the run stops at the first
    /// failure, so it is the last. `index` is the stage that stopped the run.
    StageAfterStop { index: usize },
    /// A stage report is a dry run where the first is not, or the other way round.
    PlanDiffers { index: usize },
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

/// Every exit class, to find the ones a stated code belongs to.
const CLASSES: [ExitClass; 8] = [
    ExitClass::Done,
    ExitClass::Usage,
    ExitClass::Refused,
    ExitClass::FailedNoEffect,
    ExitClass::Indeterminate,
    ExitClass::FailedChanged,
    ExitClass::ValidationFailed,
    ExitClass::Leaks,
];

/// The retry advice of an exit class: running again is safe after a done run and after a
/// failure with no effect, unsafe after a failure that changed something, and for a human to
/// decide otherwise.
pub fn default_retry(class: ExitClass) -> Retry {
    match class {
        ExitClass::Done | ExitClass::FailedNoEffect => Retry::Safe,
        ExitClass::FailedChanged => Retry::Unsafe,
        ExitClass::Usage
        | ExitClass::Refused
        | ExitClass::Indeterminate
        | ExitClass::ValidationFailed
        | ExitClass::Leaks => Retry::NeedsHuman,
    }
}

/// Checks a stage report: its schema version, the run it belongs to, what it names, every
/// action it lists, and that its exit code and retry advice are the ones its verdict, its
/// reason and its effects give.
pub fn validate_stage(report: &StageReport) -> Result<(), ReportError> {
    report_schema()
        .check(report.schema_version)
        .map_err(ReportError::Schema)?;
    check_run(report.run.as_ref())?;
    if report.stage.is_empty() {
        return Err(ReportError::StageNameEmpty);
    }
    if !report.revision.is_empty() && !is_lower_hex(&report.revision, 40) {
        return Err(ReportError::Revision {
            text: report.revision.clone(),
        });
    }
    if !report.set_hash.is_empty() && !is_lower_hex(&report.set_hash, 64) {
        return Err(ReportError::SetHash {
            text: report.set_hash.clone(),
        });
    }
    check_only(&report.only)?;
    if report.finished_at_ms < report.started_at_ms {
        return Err(ReportError::FinishedBeforeStarted {
            stage: report.stage.clone(),
            started: report.started_at_ms,
            finished: report.finished_at_ms,
        });
    }
    check_actions(&report.actions, report.plan)?;
    check_outcome(
        &Outcome {
            verdict: report.verdict,
            reason: report.reason,
            exit_code: report.exit_code,
            retry: report.retry,
        },
        landed(report.actions.iter()),
    )
}

/// Checks a run report and every stage report in it. Each stage is checked as a stage is and
/// must belong to the run. The run ends as its first stage that did not end done ended, with
/// that stage's verdict and reason; when every stage ended done it ends done. Its exit code
/// counts what any stage landed, so a failure after an earlier stage published is class 6
/// though the failing stage alone is class 4.
pub fn validate_run(report: &RunReport) -> Result<(), ReportError> {
    report_schema()
        .check(report.schema_version)
        .map_err(ReportError::Schema)?;
    let run = report.run.as_ref();
    check_run(run)?;
    if report.stages.is_empty() {
        return Err(ReportError::NoStages);
    }
    let mut names = BTreeSet::new();
    for (index, stage) in report.stages.iter().enumerate() {
        validate_stage(stage).map_err(|error| ReportError::Stage {
            index,
            error: Box::new(error),
        })?;
        if stage.run.as_ref() != run {
            return Err(ReportError::StageRunDiffers { index });
        }
        if !names.insert(stage.stage.as_str()) {
            return Err(ReportError::StageTwice {
                name: stage.stage.clone(),
            });
        }
    }
    let stated = verdict_of(report.verdict);
    match report
        .stages
        .iter()
        .find(|stage| !is_done(verdict_of(stage.verdict)))
    {
        Some(stopped) => {
            let expected = verdict_of(stopped.verdict);
            if stated != expected {
                return Err(ReportError::RunVerdictDiffers { stated, expected });
            }
            if report.reason != stopped.reason {
                return Err(ReportError::RunReasonDiffers {
                    stated: reason_of(report.reason),
                    expected: reason_of(stopped.reason),
                });
            }
        }
        None if !is_done(stated) => return Err(ReportError::RunNotDone { stated }),
        None => {}
    }
    check_outcome(
        &Outcome {
            verdict: report.verdict,
            reason: report.reason,
            exit_code: report.exit_code,
            retry: report.retry,
        },
        landed(report.stages.iter().flat_map(|stage| &stage.actions)),
    )
}

/// How a report says it ended, as the numbers it carries.
struct Outcome {
    verdict: i32,
    reason: i32,
    exit_code: u32,
    retry: i32,
}

fn verdict_of(number: i32) -> Verdict {
    Verdict::try_from(number).unwrap_or(Verdict::Unspecified)
}

fn reason_of(number: i32) -> Reason {
    Reason::try_from(number).unwrap_or(Reason::Unspecified)
}

fn is_done(verdict: Verdict) -> bool {
    matches!(
        verdict,
        Verdict::Succeeded | Verdict::Noop | Verdict::Superseded
    )
}

fn check_run(run: Option<&RunIdentity>) -> Result<(), ReportError> {
    let run = run.ok_or(ReportError::RunMissing)?;
    run_identity::check(run).map_err(ReportError::Run)
}

fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn check_only(only: &[String]) -> Result<(), ReportError> {
    let mut seen = BTreeSet::new();
    for text in only {
        // A selector is read as the command line reads it.
        Selection::parse(std::slice::from_ref(text))
            .map_err(|_| ReportError::OnlyMalformed { text: text.clone() })?;
        if !seen.insert(text.as_str()) {
            return Err(ReportError::OnlyTwice { text: text.clone() });
        }
    }
    Ok(())
}

fn check_actions(actions: &[ActionReport], plan: bool) -> Result<(), ReportError> {
    let mut names = BTreeSet::new();
    for (index, action) in actions.iter().enumerate() {
        let name = &action.name;
        if name.is_empty() {
            return Err(ReportError::ActionNameEmpty { index });
        }
        if !names.insert(name.as_str()) {
            return Err(ReportError::ActionTwice { name: name.clone() });
        }
        if verdict_of(action.verdict) == Verdict::Unspecified {
            return Err(ReportError::ActionVerdictUnspecified { name: name.clone() });
        }
        for effect in &action.effects {
            let (action, subject) = (name.clone(), effect.subject.clone());
            match effect_state(effect.state) {
                None => return Err(ReportError::EffectUnspecified { action, subject }),
                Some(state) if plan && happened(state) => {
                    return Err(ReportError::EffectInPlan { action, subject });
                }
                Some(state) if !plan && would_happen(state) => {
                    return Err(ReportError::WouldOutsidePlan { action, subject });
                }
                Some(_) => {}
            }
        }
        let rows = action
            .rows
            .iter()
            .chain(action.validations.iter().flat_map(|v| &v.rows));
        for row in rows {
            let known = RowState::try_from(row.state).is_ok_and(|s| s != RowState::RowUnspecified);
            if !known {
                return Err(ReportError::RowUnspecified {
                    action: name.clone(),
                    row: row.name.clone(),
                });
            }
        }
    }
    Ok(())
}

fn effect_state(number: i32) -> Option<EffectState> {
    EffectState::try_from(number)
        .ok()
        .filter(|state| *state != EffectState::EffectUnspecified)
}

/// States that say something was done outside the process or in the store.
fn happened(state: EffectState) -> bool {
    matches!(
        state,
        EffectState::Built
            | EffectState::Published
            | EffectState::Staged
            | EffectState::Yanked
            | EffectState::Tested
    )
}

/// States that say what a dry run would do.
fn would_happen(state: EffectState) -> bool {
    matches!(
        state,
        EffectState::WouldBuild | EffectState::WouldPublish | EffectState::WouldTest
    )
}

/// The subject of the first coordinate the actions published, staged or yanked: an external
/// effect that landed.
fn landed<'a>(actions: impl Iterator<Item = &'a ActionReport>) -> Option<&'a str> {
    actions
        .flat_map(|action| &action.effects)
        .find(|effect| {
            matches!(
                effect_state(effect.state),
                Some(EffectState::Published | EffectState::Staged | EffectState::Yanked)
            )
        })
        .map(|effect| effect.subject.as_str())
}

/// The rules every report shares about how it ended: the reason, the exit code and the retry
/// advice follow from the verdict and from whether anything landed.
fn check_outcome(outcome: &Outcome, landed: Option<&str>) -> Result<(), ReportError> {
    let verdict = verdict_of(outcome.verdict);
    if verdict == Verdict::Unspecified {
        return Err(ReportError::VerdictUnspecified);
    }
    let reason = Reason::try_from(outcome.reason).map_err(|_| ReportError::ReasonUnknown {
        number: outcome.reason,
    })?;
    match verdict {
        Verdict::Refused | Verdict::Failed | Verdict::Indeterminate | Verdict::ValidationFailed
            if reason == Reason::Unspecified =>
        {
            return Err(ReportError::ReasonRequired { verdict });
        }
        Verdict::Succeeded | Verdict::Noop if reason != Reason::Unspecified => {
            return Err(ReportError::ReasonOnSuccess { verdict, reason });
        }
        _ => {}
    }
    if let Some(effect) = landed {
        let promises = CLASSES
            .iter()
            .any(|c| u32::from(c.code()) == outcome.exit_code && c.promises_no_effect());
        if promises {
            return Err(ReportError::PromisesNoEffect {
                exit_code: outcome.exit_code,
                effect: effect.to_owned(),
            });
        }
    }
    let class = exit_class(verdict, reason, landed.is_some());
    let expected = u32::from(class.code());
    if outcome.exit_code != expected {
        return Err(ReportError::ExitCode {
            stated: outcome.exit_code,
            expected,
        });
    }
    let stated = Retry::try_from(outcome.retry)
        .ok()
        .filter(|retry| *retry != Retry::Unspecified)
        .ok_or(ReportError::RetryUnspecified)?;
    let default = default_retry(class);
    if stated != default && stated != Retry::NeedsHuman {
        return Err(ReportError::RetryWeaker { stated, default });
    }
    Ok(())
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(refusal) => write!(f, "{refusal}"),
            Self::RunMissing => write!(f, "the report names no run"),
            Self::Run(error) => write!(f, "the run identity is refused: {error}"),
            Self::StageNameEmpty => write!(f, "the stage has no name"),
            Self::Revision { text } => {
                write!(f, "revision '{text}' is not 40 lowercase hex digits")
            }
            Self::OnlyMalformed { text } => write!(
                f,
                "selector '{text}' is not action:<name> or validation:<name>"
            ),
            Self::OnlyTwice { text } => write!(f, "selector '{text}' is listed twice"),
            Self::SetHash { text } => {
                write!(f, "set hash '{text}' is not 64 lowercase hex digits")
            }
            Self::VerdictUnspecified => write!(f, "the verdict is not set"),
            Self::ReasonUnknown { number } => write!(f, "reason {number} is not known"),
            Self::ReasonRequired { verdict } => {
                write!(f, "a {verdict:?} verdict names its reason")
            }
            Self::ReasonOnSuccess { verdict, reason } => {
                write!(
                    f,
                    "a {verdict:?} verdict carries no reason, found {reason:?}"
                )
            }
            Self::ExitCode { stated, expected } => {
                write!(
                    f,
                    "exit code {stated} is not {expected}, the code this report's verdict gives"
                )
            }
            Self::PromisesNoEffect { exit_code, effect } => write!(
                f,
                "exit code {exit_code} says nothing changed, but '{effect}' landed"
            ),
            Self::RetryUnspecified => write!(f, "the retry advice is not set"),
            Self::RetryWeaker { stated, default } => write!(
                f,
                "retry advice {stated:?} is weaker than {default:?}, the advice of this exit class"
            ),
            Self::FinishedBeforeStarted {
                stage,
                started,
                finished,
            } => write!(
                f,
                "stage '{stage}' finished at {finished} ms, before it started at {started} ms"
            ),
            Self::StageAfterStop { index } => write!(
                f,
                "a stage follows stage report {index}, which did not end done; a run stops at the first failure"
            ),
            Self::PlanDiffers { index } => write!(
                f,
                "stage report {index} is a dry run where the first is not, or the other way round"
            ),
            Self::ActionNameEmpty { index } => write!(f, "action {index} has no name"),
            Self::ActionTwice { name } => write!(f, "action '{name}' is listed twice"),
            Self::ActionVerdictUnspecified { name } => {
                write!(f, "action '{name}' has no verdict")
            }
            Self::EffectUnspecified { action, subject } => {
                write!(
                    f,
                    "action '{action}': the effect on '{subject}' has no state"
                )
            }
            Self::EffectInPlan { action, subject } => {
                write!(f, "action '{action}': a dry run lists '{subject}' as done")
            }
            Self::WouldOutsidePlan { action, subject } => write!(
                f,
                "action '{action}': a real run lists '{subject}' as only planned"
            ),
            Self::RowUnspecified { action, row } => {
                write!(f, "action '{action}': row '{row}' has no state")
            }
            Self::NoStages => write!(f, "the run report lists no stage"),
            Self::StageRunDiffers { index } => {
                write!(f, "stage report {index} belongs to another run")
            }
            Self::StageTwice { name } => write!(f, "stage '{name}' is reported twice"),
            Self::Stage { index, error } => write!(f, "stage report {index}: {error}"),
            Self::RunVerdictDiffers { stated, expected } => write!(
                f,
                "the run says {stated:?}; the first stage that did not end done says {expected:?}"
            ),
            Self::RunReasonDiffers { stated, expected } => write!(
                f,
                "the run says reason {stated:?}; the first stage that did not end done says {expected:?}"
            ),
            Self::RunNotDone { stated } => {
                write!(f, "every stage ended done, but the run says {stated:?}")
            }
        }
    }
}

impl std::error::Error for ReportError {}
