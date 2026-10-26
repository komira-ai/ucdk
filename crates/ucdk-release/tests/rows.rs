// SPDX-License-Identifier: Apache-2.0
//! Accounting for the rows of a validation: faults of the rows, then the verdict.

use ucdk_release::rows::{RowFault, account_rows};
use ucdk_release_proto::v1::{ExpectedRow, Reason, Row, RowState, Verdict};

fn authored(names: &[&str]) -> Vec<ExpectedRow> {
    names
        .iter()
        .map(|n| ExpectedRow {
            name: (*n).to_owned(),
            asserts: format!("{n} holds"),
        })
        .collect()
}

fn emitted(rows: &[(&str, RowState)]) -> Vec<Row> {
    rows.iter()
        .map(|(n, s)| Row {
            name: (*n).to_owned(),
            state: *s as i32,
            detail: String::new(),
        })
        .collect()
}

use RowState::{RowFailed, RowNotReached, RowNotRun, RowPassed, RowUnreadable};

#[test]
fn truncated_run_is_a_fault() {
    let fault = account_rows(
        &authored(&["a", "b", "c"]),
        &emitted(&[("a", RowPassed), ("b", RowPassed)]),
    )
    .expect_err("a row is missing");
    assert_eq!(
        fault,
        RowFault::Truncated {
            index: 2,
            expected: "c".to_owned()
        }
    );
    assert_eq!(fault.reason(), Reason::RowAccounting);
    assert!(fault.to_string().contains("row 2"), "{fault}");
    let nothing = account_rows(&authored(&["a"]), &[]).expect_err("nothing emitted");
    assert_eq!(
        nothing,
        RowFault::Truncated {
            index: 0,
            expected: "a".to_owned()
        }
    );
}

#[test]
fn swap_at_equal_length_names_both_rows() {
    // The names have one length, so a comparison of lengths passes; the names are compared.
    let fault = account_rows(
        &authored(&["aa", "bb"]),
        &emitted(&[("bb", RowPassed), ("aa", RowPassed)]),
    )
    .expect_err("swapped");
    assert_eq!(
        fault,
        RowFault::Mismatch {
            index: 0,
            expected: "aa".to_owned(),
            got: "bb".to_owned()
        }
    );
    let text = fault.to_string();
    assert!(
        text.contains("aa") && text.contains("bb") && text.contains("row 0"),
        "{text}"
    );
}

#[test]
fn a_duplicated_row_is_a_fault_at_its_index() {
    let fault = account_rows(
        &authored(&["a", "b"]),
        &emitted(&[("a", RowPassed), ("a", RowPassed)]),
    )
    .expect_err("duplicate");
    assert_eq!(
        fault,
        RowFault::Mismatch {
            index: 1,
            expected: "b".to_owned(),
            got: "a".to_owned()
        }
    );
}

#[test]
fn a_row_that_was_not_authored_is_a_fault() {
    let fault = account_rows(
        &authored(&["a"]),
        &emitted(&[("a", RowPassed), ("extra", RowPassed)]),
    )
    .expect_err("extra");
    assert_eq!(
        fault,
        RowFault::Extra {
            index: 1,
            got: "extra".to_owned()
        }
    );
    assert!(fault.to_string().contains("extra"), "{fault}");
}

#[test]
fn an_empty_authored_list_and_a_blank_assertion_are_faults() {
    let fault = account_rows(&[], &[]).expect_err("nothing authored");
    assert_eq!(fault, RowFault::EmptyAuthored);
    assert!(!fault.to_string().is_empty());
    for blank in ["", "   ", "\n\t"] {
        let mut rows = authored(&["a", "b"]);
        rows[1].asserts = blank.to_owned();
        let fault = account_rows(&rows, &emitted(&[("a", RowPassed), ("b", RowPassed)]))
            .expect_err("blank");
        assert_eq!(fault, RowFault::BlankAsserts { index: 1 });
        assert!(fault.to_string().contains("row 1"), "{fault}");
    }
}

#[test]
fn a_row_with_no_state_or_an_unknown_state_is_a_fault() {
    for state in [0, 99] {
        let mut rows = emitted(&[("a", RowPassed), ("b", RowPassed)]);
        rows[1].state = state;
        let fault = account_rows(&authored(&["a", "b"]), &rows).expect_err("no state");
        assert_eq!(fault, RowFault::UnspecifiedState { index: 1 });
        assert!(!fault.to_string().is_empty());
    }
}

#[test]
fn a_run_that_passes_every_asserted_row_succeeds() {
    let verdict = account_rows(
        &authored(&["a", "b"]),
        &emitted(&[("a", RowPassed), ("b", RowPassed)]),
    );
    assert_eq!(verdict, Ok(Verdict::Succeeded));
}

#[test]
fn a_failed_row_fails_the_validation_whatever_else_happened() {
    for others in [RowPassed, RowUnreadable, RowNotReached, RowNotRun] {
        let verdict = account_rows(
            &authored(&["a", "b"]),
            &emitted(&[("a", RowFailed), ("b", others)]),
        );
        assert_eq!(verdict, Ok(Verdict::ValidationFailed), "{others:?}");
    }
}

#[test]
fn a_row_not_reached_or_unreadable_is_never_a_pass() {
    for state in [RowNotReached, RowUnreadable] {
        let verdict = account_rows(
            &authored(&["a", "b"]),
            &emitted(&[("a", RowPassed), ("b", state)]),
        );
        assert_eq!(verdict, Ok(Verdict::Indeterminate), "{state:?}");
    }
}

#[test]
fn a_row_left_unrun_is_not_asserted_and_nothing_asserted_is_a_fault() {
    let skipped = account_rows(
        &authored(&["a", "b"]),
        &emitted(&[("a", RowPassed), ("b", RowNotRun)]),
    );
    assert_eq!(skipped, Ok(Verdict::Succeeded));
    let fault = account_rows(
        &authored(&["a", "b"]),
        &emitted(&[("a", RowNotRun), ("b", RowNotRun)]),
    )
    .expect_err("nothing asserted");
    assert_eq!(fault, RowFault::NothingAsserted);
    assert_eq!(fault.reason(), Reason::RowAccounting);
    assert!(!fault.to_string().is_empty());
}
