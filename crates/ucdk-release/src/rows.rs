// SPDX-License-Identifier: Apache-2.0
//! Accounting for the rows of a validation or a Test action. The rows are derived before
//! anything is read (one per member, one per coordinate, one per test target), and what was
//! evaluated is compared with that list index by index, because a count check alone passes a
//! truncated run.

use std::fmt;

use ucdk_release_proto::v1::{Reason, Row, RowState, Verdict};

/// A row the engine derived before it read anything: what it is about, and the one sentence it
/// asserts. The name of its row is `<subject>: <assertion>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedRow {
    pub subject: String,
    pub assertion: String,
}

impl ExpectedRow {
    pub fn new(subject: impl Into<String>, assertion: impl Into<String>) -> Self {
        ExpectedRow {
            subject: subject.into(),
            assertion: assertion.into(),
        }
    }

    /// The name an evaluated row of this one carries.
    pub fn name(&self) -> String {
        format!("{}: {}", self.subject, self.assertion)
    }
}

/// How the evaluated rows differ from the derived ones, at the first index where they do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowFault {
    /// Nothing was derived, so nothing could be asserted.
    NothingDerived,
    /// The derived row at `index` was not evaluated: the run stopped short or skipped it.
    Missing { index: usize, expected: String },
    /// The row at `index` is the one derived at `index + 1`, and the next row is the one
    /// derived at `index`.
    Swapped {
        index: usize,
        expected: String,
        found: String,
    },
    /// The row at `index` repeats an earlier row.
    Duplicate { index: usize, found: String },
    /// The row at `index` is none of the derived rows.
    NotDerived { index: usize, found: String },
}

impl RowFault {
    /// The reason a verdict carries.
    pub fn reason(&self) -> Reason {
        Reason::RowAccounting
    }
}

impl fmt::Display for RowFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RowFault::NothingDerived => f.write_str("no row was derived, so nothing was asserted"),
            RowFault::Missing { index, expected } => {
                write!(
                    f,
                    "row {index} should be {expected:?} and was not evaluated"
                )
            }
            RowFault::Swapped {
                index,
                expected,
                found,
            } => write!(
                f,
                "rows {index} and {} are swapped: {found:?} came where {expected:?} was derived",
                index + 1
            ),
            RowFault::Duplicate { index, found } => {
                write!(f, "row {index} {found:?} repeats an earlier row")
            }
            RowFault::NotDerived { index, found } => {
                write!(f, "row {index} {found:?} is none of the derived rows")
            }
        }
    }
}

/// Compares the evaluated `rows` with the derived `expected` rows, index by index, and gives
/// the verdict they come to.
///
/// A fault is the first index at which the two lists differ. Otherwise the verdict is
/// `Succeeded` only when every row passed. The first row that did not pass decides: a failed
/// row, a row not run or not reached, and a row of no state are `ValidationFailed` (none of
/// them is ever a pass), and a row that could not be read is `Indeterminate`.
pub fn account_rows(expected: &[ExpectedRow], rows: &[Row]) -> Result<Verdict, RowFault> {
    if expected.is_empty() {
        return Err(RowFault::NothingDerived);
    }
    let names: Vec<String> = expected.iter().map(ExpectedRow::name).collect();
    for (index, want) in names.iter().enumerate() {
        let Some(row) = rows.get(index) else {
            return Err(RowFault::Missing {
                index,
                expected: want.clone(),
            });
        };
        if &row.name == want {
            continue;
        }
        let found = row.name.clone();
        return Err(if rows[..index].iter().any(|r| r.name == found) {
            RowFault::Duplicate { index, found }
        } else if names.get(index + 1) == Some(&found)
            && rows.get(index + 1).map(|r| &r.name) == Some(want)
        {
            RowFault::Swapped {
                index,
                expected: want.clone(),
                found,
            }
        } else if names.contains(&found) {
            RowFault::Missing {
                index,
                expected: want.clone(),
            }
        } else {
            RowFault::NotDerived { index, found }
        });
    }
    if let Some(extra) = rows.get(expected.len()) {
        let (index, found) = (expected.len(), extra.name.clone());
        return Err(if rows[..index].iter().any(|r| r.name == found) {
            RowFault::Duplicate { index, found }
        } else {
            RowFault::NotDerived { index, found }
        });
    }
    Ok(rows
        .iter()
        .find(|row| row.state != RowState::RowPassed as i32)
        .map_or(Verdict::Succeeded, |row| {
            if row.state == RowState::RowUnreadable as i32 {
                Verdict::Indeterminate
            } else {
                Verdict::ValidationFailed
            }
        }))
}
