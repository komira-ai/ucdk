// SPDX-License-Identifier: Apache-2.0
//! Turning the rows a validator emitted into a verdict. The rows are authored up front and are
//! the denominator: emitted names are compared with them index by index, because a count alone
//! passes a truncated run.

use std::fmt;

use ucdk_release_proto::v1::{ExpectedRow, Reason, Row, RowState, Verdict};

/// A fault of the rows themselves, not of the subject they read: a spec fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowFault {
    /// The validation authored no rows.
    EmptyAuthored,
    /// An authored row says nothing about what it proves.
    BlankAsserts { index: usize },
    /// The validator emitted fewer rows than were authored; `index` is the first missing.
    Truncated { index: usize, expected: String },
    /// The validator emitted more rows than were authored; `index` is the first extra.
    Extra { index: usize, got: String },
    /// The row at `index` is not the authored one: a swap, a duplicate or another name.
    Mismatch {
        index: usize,
        expected: String,
        got: String,
    },
    /// The row at `index` has no state, or a state this reader does not know.
    UnspecifiedState { index: usize },
    /// No row was asserted: every row was left unrun.
    NothingAsserted,
}

impl RowFault {
    pub fn reason(&self) -> Reason {
        Reason::RowAccounting
    }
}

impl fmt::Display for RowFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RowFault::EmptyAuthored => f.write_str("the validation authors no rows"),
            RowFault::BlankAsserts { index } => {
                write!(f, "authored row {index} says nothing about what it proves")
            }
            RowFault::Truncated { index, expected } => {
                write!(f, "the validator stopped before row {index}, {expected:?}")
            }
            RowFault::Extra { index, got } => write!(
                f,
                "the validator emitted row {index}, {got:?}, which was not authored"
            ),
            RowFault::Mismatch {
                index,
                expected,
                got,
            } => {
                write!(
                    f,
                    "row {index} is {got:?}, and the authored row is {expected:?}"
                )
            }
            RowFault::UnspecifiedState { index } => write!(f, "row {index} has no known state"),
            RowFault::NothingAsserted => f.write_str("no row was asserted"),
        }
    }
}

/// The verdict of a validation: `Succeeded` only with at least one asserted row and every
/// asserted row passed. A failed row is `ValidationFailed`; a row not reached or unreadable is
/// `Indeterminate`, never a pass. A row left unrun is not asserted.
pub fn account_rows(authored: &[ExpectedRow], emitted: &[Row]) -> Result<Verdict, RowFault> {
    if authored.is_empty() {
        return Err(RowFault::EmptyAuthored);
    }
    if let Some(index) = authored.iter().position(|r| r.asserts.trim().is_empty()) {
        return Err(RowFault::BlankAsserts { index });
    }
    for (index, want) in authored.iter().enumerate() {
        match emitted.get(index) {
            None => {
                return Err(RowFault::Truncated {
                    index,
                    expected: want.name.clone(),
                });
            }
            Some(got) if got.name != want.name => {
                return Err(RowFault::Mismatch {
                    index,
                    expected: want.name.clone(),
                    got: got.name.clone(),
                });
            }
            Some(_) => {}
        }
    }
    if let Some(extra) = emitted.get(authored.len()) {
        return Err(RowFault::Extra {
            index: authored.len(),
            got: extra.name.clone(),
        });
    }
    let mut states = Vec::new();
    for (index, row) in emitted.iter().enumerate() {
        match RowState::try_from(row.state) {
            Ok(RowState::RowUnspecified) | Err(_) => {
                return Err(RowFault::UnspecifiedState { index });
            }
            Ok(state) => states.push(state),
        }
    }
    let asserted: Vec<RowState> = states
        .into_iter()
        .filter(|s| *s != RowState::RowNotRun)
        .collect();
    if asserted.is_empty() {
        return Err(RowFault::NothingAsserted);
    }
    Ok(if asserted.contains(&RowState::RowFailed) {
        Verdict::ValidationFailed
    } else if asserted.iter().any(|s| *s != RowState::RowPassed) {
        Verdict::Indeterminate
    } else {
        Verdict::Succeeded
    })
}
