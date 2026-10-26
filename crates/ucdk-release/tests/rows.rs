// SPDX-License-Identifier: Apache-2.0
//! Accounting for the rows of a validation against the rows derived before it read anything.

use ucdk_release::rows::{ExpectedRow, RowFault, account_rows};
use ucdk_release_proto::v1::{Reason, Row, RowState, Verdict};

fn expected(subjects: &[&str]) -> Vec<ExpectedRow> {
    subjects
        .iter()
        .map(|s| ExpectedRow::new(*s, "it holds"))
        .collect()
}

fn row(subject: &str, state: RowState) -> Row {
    Row {
        name: ExpectedRow::new(subject, "it holds").name(),
        state: state as i32,
        detail: String::new(),
    }
}

fn passed(subjects: &[&str]) -> Vec<Row> {
    subjects
        .iter()
        .map(|s| row(s, RowState::RowPassed))
        .collect()
}

#[test]
fn the_name_of_a_row_is_its_subject_and_its_assertion() {
    assert_eq!(
        ExpectedRow::new("tool", "it holds").name(),
        "tool: it holds"
    );
}

#[test]
fn every_row_passed_is_succeeded() {
    assert_eq!(
        account_rows(&expected(&["a", "b", "c"]), &passed(&["a", "b", "c"])),
        Ok(Verdict::Succeeded)
    );
    // One asserted row is enough.
    assert_eq!(
        account_rows(&expected(&["a"]), &passed(&["a"])),
        Ok(Verdict::Succeeded)
    );
}

#[test]
fn no_derived_row_is_a_fault_even_when_nothing_was_evaluated() {
    let fault = account_rows(&[], &[]).expect_err("nothing asserted");
    assert_eq!(fault, RowFault::NothingDerived);
    assert_eq!(fault.reason(), Reason::RowAccounting);
    assert!(fault.to_string().contains("no row was derived"), "{fault}");
    // Rows that nothing derived are not rescued by being there.
    assert_eq!(
        account_rows(&[], &passed(&["a"])),
        Err(RowFault::NothingDerived)
    );
}

#[test]
fn truncated_run_is_a_fault() {
    // Two of three evaluated: a count of passed rows would say all passed.
    let fault = account_rows(&expected(&["a", "b", "c"]), &passed(&["a", "b"])).expect_err("short");
    assert_eq!(
        fault,
        RowFault::Missing {
            index: 2,
            expected: "c: it holds".to_owned()
        }
    );
    assert_eq!(fault.reason(), Reason::RowAccounting);
    assert!(fault.to_string().contains("row 2") && fault.to_string().contains("c: it holds"));
    // Nothing evaluated names the first row.
    assert_eq!(
        account_rows(&expected(&["a", "b"]), &[]),
        Err(RowFault::Missing {
            index: 0,
            expected: "a: it holds".to_owned()
        })
    );
    // A row skipped in the middle names the index of the skipped one.
    assert_eq!(
        account_rows(&expected(&["a", "b", "c"]), &passed(&["a", "c"])),
        Err(RowFault::Missing {
            index: 1,
            expected: "b: it holds".to_owned()
        })
    );
}

#[test]
fn swap_at_equal_length_names_both_rows() {
    // Same count, same names, two of them exchanged: only a comparison by index sees it.
    let fault =
        account_rows(&expected(&["a", "b", "c"]), &passed(&["b", "a", "c"])).expect_err("swapped");
    assert_eq!(
        fault,
        RowFault::Swapped {
            index: 0,
            expected: "a: it holds".to_owned(),
            found: "b: it holds".to_owned()
        }
    );
    assert_eq!(fault.reason(), Reason::RowAccounting);
    let text = fault.to_string();
    assert!(
        text.contains("rows 0 and 1")
            && text.contains("b: it holds")
            && text.contains("a: it holds"),
        "{text}"
    );
    // A swap further in names the first of the pair.
    assert!(matches!(
        account_rows(&expected(&["a", "b", "c"]), &passed(&["a", "c", "b"])),
        Err(RowFault::Swapped { index: 1, .. })
    ));
    // A rotation of three is not a swap: the first row is missing where it was derived.
    assert!(matches!(
        account_rows(&expected(&["a", "b", "c"]), &passed(&["b", "c", "a"])),
        Err(RowFault::Missing { index: 0, .. })
    ));
}

#[test]
fn a_row_that_was_not_derived_is_a_fault() {
    let fault = account_rows(&expected(&["a", "b"]), &passed(&["a", "x"])).expect_err("unknown");
    assert_eq!(
        fault,
        RowFault::NotDerived {
            index: 1,
            found: "x: it holds".to_owned()
        }
    );
    assert!(fault.to_string().contains("none of the derived rows"));
    // An extra row after the derived ones is one too.
    assert_eq!(
        account_rows(&expected(&["a"]), &passed(&["a", "x"])),
        Err(RowFault::NotDerived {
            index: 1,
            found: "x: it holds".to_owned()
        })
    );
    // A row with the right subject and another assertion is not the derived row either.
    let mut other = passed(&["a"]);
    other[0].name = "a: something else".to_owned();
    assert!(matches!(
        account_rows(&expected(&["a"]), &other),
        Err(RowFault::NotDerived { index: 0, .. })
    ));
}

#[test]
fn a_repeated_row_is_a_fault() {
    let fault = account_rows(&expected(&["a", "b"]), &passed(&["a", "a"])).expect_err("repeat");
    assert_eq!(
        fault,
        RowFault::Duplicate {
            index: 1,
            found: "a: it holds".to_owned()
        }
    );
    assert!(fault.to_string().contains("repeats"));
    // A repeat after the derived rows is a repeat, not an unknown row.
    assert!(matches!(
        account_rows(&expected(&["a", "b"]), &passed(&["a", "b", "a"])),
        Err(RowFault::Duplicate { index: 2, .. })
    ));
}

#[test]
fn a_row_that_did_not_pass_is_never_a_success() {
    let run = |states: &[RowState]| {
        let subjects = ["a", "b", "c"];
        let rows: Vec<Row> = states
            .iter()
            .zip(subjects)
            .map(|(s, n)| row(n, *s))
            .collect();
        account_rows(&expected(&subjects[..states.len()]), &rows)
    };
    use RowState::{RowFailed, RowNotReached, RowNotRun, RowPassed, RowUnreadable, RowUnspecified};
    assert_eq!(
        run(&[RowPassed, RowFailed, RowNotReached]),
        Ok(Verdict::ValidationFailed)
    );
    for state in [RowFailed, RowNotRun, RowNotReached, RowUnspecified] {
        assert_eq!(
            run(&[RowPassed, state]),
            Ok(Verdict::ValidationFailed),
            "{state:?}"
        );
    }
    // A row that could not be read is `INDETERMINATE`, and never a pass.
    assert_eq!(run(&[RowPassed, RowUnreadable]), Ok(Verdict::Indeterminate));
    // The first row that did not pass decides.
    assert_eq!(
        run(&[RowFailed, RowUnreadable]),
        Ok(Verdict::ValidationFailed)
    );
    assert_eq!(run(&[RowUnreadable, RowFailed]), Ok(Verdict::Indeterminate));
    assert_eq!(
        run(&[RowPassed, RowPassed, RowPassed]),
        Ok(Verdict::Succeeded)
    );
}
