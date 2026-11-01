// SPDX-License-Identifier: Apache-2.0
//! Accounting for the rows of a validation or a Test action. The rows are derived before
//! anything is read (one per member, one per coordinate, one per test target), and what was
//! evaluated is compared with that list index by index, because a count check alone passes a
//! truncated run.

use std::collections::BTreeSet;
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
    /// The derived row at `index` has a blank subject.
    BlankSubject { index: usize },
    /// The derived row at `index` has a blank assertion.
    BlankAssertion { index: usize, subject: String },
    /// The derived row at `index` has the name of an earlier derived row.
    DuplicateSpec { index: usize, name: String },
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
            RowFault::BlankSubject { index } => {
                write!(
                    f,
                    "derived row {index} has a blank subject, so it names nothing"
                )
            }
            RowFault::BlankAssertion { index, subject } => write!(
                f,
                "derived row {index} ({subject:?}) has a blank assertion, so it asserts nothing"
            ),
            RowFault::DuplicateSpec { index, name } => {
                write!(
                    f,
                    "derived row {index} {name:?} repeats an earlier derived row"
                )
            }
        }
    }
}

/// Why the derived `expected` rows cannot be the authority for an accounting, if they cannot.
///
/// An accounting compares what was evaluated with what was derived, so a derived list that is
/// itself broken makes a gate that passes. It is broken when it is empty (it accepts every run,
/// including one that evaluated nothing), when a row has a blank subject (it can never be told
/// from another row), when a row has a blank assertion (dropping it would remove no stated
/// obligation, only a number), and when two rows have one name (a failure could not say which
/// one it is about). Blank means empty or only whitespace. A blank is reported before a
/// repeat, and the first of either, by index, is the one returned.
pub fn spec_fault(expected: &[ExpectedRow]) -> Option<RowFault> {
    if expected.is_empty() {
        return Some(RowFault::NothingDerived);
    }
    for (index, row) in expected.iter().enumerate() {
        if row.subject.trim().is_empty() {
            return Some(RowFault::BlankSubject { index });
        }
        if row.assertion.trim().is_empty() {
            return Some(RowFault::BlankAssertion {
                index,
                subject: row.subject.clone(),
            });
        }
    }
    // A `BTreeSet`, because this crate holds no `HashSet`: it makes the repeat check n log n.
    let mut seen = BTreeSet::new();
    expected.iter().enumerate().find_map(|(index, row)| {
        let name = row.name();
        (!seen.insert(name.clone())).then_some(RowFault::DuplicateSpec { index, name })
    })
}

/// Compares the evaluated `rows` with the derived `expected` rows, index by index, and gives
/// the verdict they come to.
///
/// A derived list with a [`spec_fault`] is refused first, whatever the rows are. Then a fault is
/// the first index at which the two lists differ. With none, the verdict is `Succeeded` only
/// when every row passed. The first row that did not pass decides: a failed
/// row, a row not run or not reached, and a row of no state are `ValidationFailed` (none of
/// them is ever a pass), and a row that could not be read is `Indeterminate`.
pub fn account_rows(expected: &[ExpectedRow], rows: &[Row]) -> Result<Verdict, RowFault> {
    if let Some(fault) = spec_fault(expected) {
        return Err(fault);
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
