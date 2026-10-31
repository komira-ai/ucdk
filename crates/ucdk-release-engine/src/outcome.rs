// SPDX-License-Identifier: Apache-2.0
//! What a request answers: a verdict, the reason when it did not pass, a sentence for the
//! operator, and the report of each action and validation that ran.

use ucdk_ledger::{GuardError, LedgerError};
use ucdk_release::codec::CodecError;
use ucdk_release::rows::RowFault;
use ucdk_release_proto::v1::{ActionReport, Reason, Row, RowState, ValidationReport, Verdict};

/// The answer to a [`Request`](crate::Request).
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// How the request ended. `Succeeded` or `Noop` means everything it asked for passed, and
    /// anything else stopped it: the items after the one that stopped it were not run.
    pub verdict: Verdict,
    /// Why it did not pass; `Unspecified` for a pass.
    pub reason: Reason,
    /// What to tell the operator when it did not pass; empty for a pass.
    pub message: String,
    /// The Test actions that ran or were skipped, in the order they were reached.
    pub actions: Vec<ActionReport>,
    /// The validations that ran or were skipped, in the order they were reached.
    pub validations: Vec<ValidationReport>,
}

impl Outcome {
    /// A request that stopped before it ran anything.
    pub(crate) fn halted(verdict: Verdict, reason: Reason, message: impl Into<String>) -> Self {
        Self {
            verdict,
            reason,
            message: message.into(),
            actions: Vec::new(),
            validations: Vec::new(),
        }
    }
}

/// What one item of a request came to.
pub(crate) enum Report {
    Action(ActionReport),
    Validation(ValidationReport),
}

/// One item finished, passed or not.
pub(crate) struct Finished {
    pub(crate) verdict: Verdict,
    pub(crate) reason: Reason,
    pub(crate) message: String,
    /// Boxed, so a stop that carries no report is small: it travels in every `Err`.
    pub(crate) report: Option<Box<Report>>,
}

/// An item either finishes or stops the request with a finished item of its own, so `?` carries
/// a stop out of any depth.
pub(crate) type Flow<T> = Result<T, Finished>;

impl Finished {
    /// Whether the request goes on to the next item.
    pub(crate) fn goes_on(&self) -> bool {
        matches!(self.verdict, Verdict::Succeeded | Verdict::Noop)
    }

    /// A stop that has no report: nothing was run.
    pub(crate) fn stopped(verdict: Verdict, reason: Reason, message: impl Into<String>) -> Self {
        Self {
            verdict,
            reason,
            message: message.into(),
            report: None,
        }
    }
}

/// How a lease or a write that failed ends a request.
impl From<GuardError> for Finished {
    fn from(error: GuardError) -> Self {
        let (verdict, reason) = match error {
            GuardError::Held { .. } => (Verdict::Refused, Reason::LeaseHeld),
            GuardError::Lost => (Verdict::Interrupted, Reason::LeaseLost),
            GuardError::Unconfirmed { .. } => (Verdict::Indeterminate, Reason::LeaseUnconfirmed),
            GuardError::Ledger(_) | GuardError::Busy | GuardError::NotHeld => {
                (Verdict::Failed, Reason::Ledger)
            }
        };
        Self::stopped(verdict, reason, error.to_string())
    }
}

/// A ledger that could not be read.
impl From<LedgerError> for Finished {
    fn from(error: LedgerError) -> Self {
        Self::stopped(Verdict::Failed, Reason::Ledger, error.to_string())
    }
}

/// A record the codec refuses, to write or to read.
impl From<CodecError> for Finished {
    fn from(error: CodecError) -> Self {
        Self::stopped(Verdict::Failed, Reason::Ledger, error.to_string())
    }
}

/// A fault in the accounting of rows (nothing derived, a row missing or out of place) is a spec
/// fault, and never a pass.
impl From<RowFault> for Finished {
    fn from(fault: RowFault) -> Self {
        Self::stopped(Verdict::Refused, fault.reason(), fault.to_string())
    }
}

pub(crate) fn row(name: &str, state: RowState, detail: impl Into<String>) -> Row {
    Row {
        name: name.to_owned(),
        state: state as i32,
        detail: detail.into(),
    }
}

/// The report of a Test action.
pub(crate) fn action_report(
    name: &str,
    verdict: Verdict,
    reason: Reason,
    message: &str,
    rows: Vec<Row>,
) -> ActionReport {
    ActionReport {
        name: name.to_owned(),
        verdict: verdict as i32,
        reason: reason as i32,
        message: message.to_owned(),
        effects: Vec::new(),
        validations: Vec::new(),
        new_names: Vec::new(),
        rows,
    }
}

/// The report of a validation.
pub(crate) fn validation_report(name: &str, verdict: Verdict, rows: Vec<Row>) -> ValidationReport {
    ValidationReport {
        name: name.to_owned(),
        verdict: verdict as i32,
        rows,
    }
}

/// The rows that did not pass, one per line of a sentence, each as `name: detail`; rows that
/// were never reached are not what went wrong.
pub(crate) fn what_failed(rows: &[Row]) -> String {
    rows.iter()
        .filter(|row| {
            row.state != RowState::RowPassed as i32 && row.state != RowState::RowNotReached as i32
        })
        .map(|row| format!("{} ({})", row.name, row.detail))
        .collect::<Vec<_>>()
        .join("; ")
}
