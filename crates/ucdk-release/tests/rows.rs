// SPDX-License-Identifier: Apache-2.0
//! Accounting for the rows of a validation against the rows derived before it read anything.

use ucdk_release::rows::{ExpectedRow, RowFault, account_rows, spec_fault};
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

fn spec(rows: &[(&str, &str)]) -> Vec<ExpectedRow> {
    rows.iter().map(|(s, a)| ExpectedRow::new(*s, *a)).collect()
}

#[test]
fn a_well_formed_spec_has_no_fault() {
    assert_eq!(spec_fault(&expected(&["a", "b", "c"])), None);
    // The same subject under two assertions, or two subjects under one, are different rows.
    assert_eq!(
        spec_fault(&spec(&[
            ("a", "it holds"),
            ("a", "it is fast"),
            ("b", "it holds")
        ])),
        None
    );
}

#[test]
fn an_empty_spec_is_a_fault() {
    assert_eq!(spec_fault(&[]), Some(RowFault::NothingDerived));
}

#[test]
fn a_blank_subject_is_a_fault_at_its_index() {
    for blank in ["", " ", "\t\n  "] {
        assert_eq!(
            spec_fault(&spec(&[("a", "it holds"), (blank, "it holds")])),
            Some(RowFault::BlankSubject { index: 1 }),
            "{blank:?}"
        );
    }
}

#[test]
fn a_blank_assertion_is_a_fault_naming_its_subject() {
    for blank in ["", " ", "\t\n  "] {
        assert_eq!(
            spec_fault(&spec(&[("a", "it holds"), ("b", blank)])),
            Some(RowFault::BlankAssertion {
                index: 1,
                subject: "b".to_owned()
            }),
            "{blank:?}"
        );
    }
}

#[test]
fn a_repeated_row_name_is_a_fault_at_the_second_one() {
    assert_eq!(
        spec_fault(&expected(&["a", "b", "a"])),
        Some(RowFault::DuplicateSpec {
            index: 2,
            name: "a: it holds".to_owned()
        })
    );
}

#[test]
fn a_blank_is_reported_before_a_repeat() {
    // The repeat is at index 1, the blank assertion at index 2: the blank still comes first.
    assert_eq!(
        spec_fault(&spec(&[("a", "x"), ("a", "x"), ("b", "")])),
        Some(RowFault::BlankAssertion {
            index: 2,
            subject: "b".to_owned()
        })
    );
}

#[test]
fn accounting_refuses_a_spec_with_a_fault_whatever_the_rows_are() {
    let blank_subject = spec(&[("a", "it holds"), ("", "it holds")]);
    let rows = vec![
        Row {
            name: blank_subject[0].name(),
            state: RowState::RowPassed as i32,
            detail: String::new(),
        },
        Row {
            name: blank_subject[1].name(),
            state: RowState::RowPassed as i32,
            detail: String::new(),
        },
    ];
    assert_eq!(
        account_rows(&blank_subject, &rows),
        Err(RowFault::BlankSubject { index: 1 })
    );
    let blank_assertion = spec(&[("a", "")]);
    assert_eq!(
        account_rows(&blank_assertion, &[]),
        Err(RowFault::BlankAssertion {
            index: 0,
            subject: "a".to_owned()
        })
    );
    let repeated = expected(&["a", "a"]);
    assert_eq!(
        account_rows(&repeated, &passed(&["a", "a"])),
        Err(RowFault::DuplicateSpec {
            index: 1,
            name: "a: it holds".to_owned()
        })
    );
}

#[test]
fn a_spec_fault_is_a_row_accounting_reason_and_reads_as_a_sentence() {
    let faults = [
        RowFault::BlankSubject { index: 3 },
        RowFault::BlankAssertion {
            index: 4,
            subject: "tool".to_owned(),
        },
        RowFault::DuplicateSpec {
            index: 5,
            name: "tool: it holds".to_owned(),
        },
    ];
    for fault in &faults {
        assert_eq!(fault.reason(), Reason::RowAccounting);
    }
    assert_eq!(
        faults[0].to_string(),
        "derived row 3 has a blank subject, so it names nothing"
    );
    assert_eq!(
        faults[1].to_string(),
        "derived row 4 (\"tool\") has a blank assertion, so it asserts nothing"
    );
    assert_eq!(
        faults[2].to_string(),
        "derived row 5 \"tool: it holds\" repeats an earlier derived row"
    );
}
