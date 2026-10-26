// SPDX-License-Identifier: Apache-2.0
//! Reading the report of a test invocation: how many tests ran, over transcripts of the two
//! report files.

use ucdk_release::test_report::ran;
use ucdk_release_proto::v1::BuildSystemKind::{Bazel, Buck2, Unspecified};

/// A Buck2 event log line that carries a test result of the given status.
fn buck2_result(status: &str) -> String {
    format!(
        r#"{{"Event":{{"event_time":"2026-10-20T10:00:00Z","data":{{"Instant":{{"data":{{"TestResult":{{"name":"t","status":"{status}","duration":1}}}}}}}}}}}}"#
    )
}

/// A Bazel build event line of a test summary.
fn bazel_summary(count: u64, status: &str) -> String {
    format!(
        r#"{{"id":{{"testSummary":{{"label":"//release:set_test"}}}},"testSummary":{{"totalRunCount":{count},"overallStatus":"{status}"}}}}"#
    )
}

fn lines(items: &[String]) -> Vec<u8> {
    items.join("\n").into_bytes()
}

#[test]
fn a_buck2_event_log_counts_results_that_are_not_skips() {
    let other = r#"{"Event":{"data":{"SpanStart":{"data":{"Test":{"suite":"s"}}}}}}"#;
    let log = lines(&[
        other.to_owned(),
        buck2_result("PASS"),
        buck2_result("FAIL"),
        buck2_result("SKIP"),
        buck2_result("OMITTED"),
        buck2_result("LISTING_SUCCESS"),
        buck2_result("LISTING_FAILED"),
        buck2_result("TIMEOUT"),
        buck2_result("FATAL"),
        other.to_owned(),
    ]);
    // PASS, FAIL, TIMEOUT and FATAL are results of a test that ran; a skip, an omission and a
    // listing are not.
    assert_eq!(ran(Buck2, Some(&log)), 4);
    // Only skips and listings: nothing ran.
    let none = lines(&[
        buck2_result("SKIP"),
        buck2_result("LISTING_SUCCESS"),
        other.to_owned(),
    ]);
    assert_eq!(ran(Buck2, Some(&none)), 0);
    // A line counts once, however many records it holds, and a record may be anywhere in it.
    let two = r#"{"a":[{"TestResult":{"status":"PASS"}},{"TestResult":{"status":"PASS"}}]}"#;
    assert_eq!(ran(Buck2, Some(two.as_bytes())), 1);
    // A record that names no status, or a status that is not a name, is not counted; and a
    // `TestResult` key that is not a record of a run does not hide a later one on the line.
    for line in [
        r#"{"TestResult":{}}"#,
        r#"{"TestResult":{"status":0}}"#,
        r#"{"TestResult":"PASS"}"#,
        r#"{"data":["TestResult"]}"#,
        r#""TestResult""#,
    ] {
        assert_eq!(ran(Buck2, Some(line.as_bytes())), 0, "{line}");
    }
    let skipped_then_run =
        r#"{"TestResult":{"status":"SKIP"},"inner":{"TestResult":{"status":"PASS"}}}"#;
    assert_eq!(ran(Buck2, Some(skipped_then_run.as_bytes())), 1);
}

#[test]
fn only_the_statuses_that_mean_a_test_ran_count_in_a_buck2_event_log() {
    // Each status that means a test ran counts once.
    for status in ["PASS", "FAIL", "FATAL", "TIMEOUT", "RERUN"] {
        assert_eq!(
            ran(Buck2, Some(buck2_result(status).as_bytes())),
            1,
            "{status}"
        );
    }
    // Every other status is not a run: the ones Buck2 names that are not a run, a name it may
    // add later, an empty name, and a run status in another case.
    for status in [
        "SKIP",
        "OMITTED",
        "LISTING_SUCCESS",
        "LISTING_FAILED",
        "UNKNOWN",
        "INFRA_FAILURE",
        "NOT_SET_TEST_STATUS",
        "NOT_SET",
        "banana",
        "skip",
        "pass",
        "Pass",
        "",
    ] {
        assert_eq!(
            ran(Buck2, Some(buck2_result(status).as_bytes())),
            0,
            "{status:?}"
        );
    }
    // A report of such results has not run a test, however many lines it holds.
    let none = lines(&[
        buck2_result("banana"),
        buck2_result("NOT_SET"),
        buck2_result("skip"),
    ]);
    assert_eq!(ran(Buck2, Some(&none)), 0);
}

#[test]
fn a_bazel_event_file_counts_the_runs_of_each_test_summary() {
    let build_event =
        r#"{"id":{"targetCompleted":{"label":"//release:set_test"}},"completed":{"success":true}}"#;
    let file = lines(&[
        build_event.to_owned(),
        bazel_summary(1, "PASSED"),
        bazel_summary(3, "FAILED"),
        bazel_summary(2, "FLAKY"),
        build_event.to_owned(),
    ]);
    assert_eq!(ran(Bazel, Some(&file)), 6);
    // The `testSummary` that counts is the event's own, not the one in its id.
    let id_only = r#"{"id":{"testSummary":{"label":"//a:b"}}}"#;
    assert_eq!(ran(Bazel, Some(id_only.as_bytes())), 0);
    // The sum does not overflow.
    let big = lines(&[
        bazel_summary(u64::from(u32::MAX), "PASSED"),
        bazel_summary(u64::from(u32::MAX), "PASSED"),
    ]);
    assert_eq!(ran(Bazel, Some(&big)), u32::MAX);
    let huge = lines(&[
        bazel_summary(u64::MAX, "PASSED"),
        bazel_summary(5, "PASSED"),
    ]);
    assert_eq!(ran(Bazel, Some(&huge)), u32::MAX);
}

#[test]
fn a_bazel_summary_that_tested_nothing_counts_no_run() {
    for status in ["NO_STATUS", "FAILED_TO_BUILD", "TOOL_HALTED_BEFORE_TESTING"] {
        let file = lines(&[bazel_summary(2, status)]);
        assert_eq!(ran(Bazel, Some(&file)), 0, "{status}");
    }
    // A summary with no status says `NO_STATUS`, which JSON leaves out; one without a count ran
    // nothing; neither is a number the reader may invent.
    let no_status = r#"{"testSummary":{"totalRunCount":4}}"#;
    assert_eq!(ran(Bazel, Some(no_status.as_bytes())), 0);
    let no_count = r#"{"testSummary":{"overallStatus":"PASSED"}}"#;
    assert_eq!(ran(Bazel, Some(no_count.as_bytes())), 0);
    let not_a_number = r#"{"testSummary":{"totalRunCount":"2","overallStatus":"PASSED"}}"#;
    assert_eq!(ran(Bazel, Some(not_a_number.as_bytes())), 0);
    let status_not_text = r#"{"testSummary":{"totalRunCount":2,"overallStatus":1}}"#;
    assert_eq!(ran(Bazel, Some(status_not_text.as_bytes())), 0);
    // One that tested nothing does not take the ones that did with it.
    let mixed = lines(&[bazel_summary(5, "NO_STATUS"), bazel_summary(2, "PASSED")]);
    assert_eq!(ran(Bazel, Some(&mixed)), 2);
}

#[test]
fn a_missing_or_unparseable_report_is_zero_cases() {
    for kind in [Buck2, Bazel] {
        assert_eq!(ran(kind, None), 0);
        assert_eq!(ran(kind, Some(b"")), 0);
        assert_eq!(ran(kind, Some(b"\n\n")), 0);
        assert_eq!(ran(kind, Some(b"not json at all")), 0);
        assert_eq!(ran(kind, Some(b"{\"truncated\":")), 0);
        assert_eq!(ran(kind, Some(b"[1, 2, 3]")), 0);
        assert_eq!(ran(kind, Some(b"\xff\xfe")), 0);
    }
    // A line that is not JSON is not counted and does not undo the lines that are.
    let mixed = lines(&[
        "garbage".to_owned(),
        buck2_result("PASS"),
        "{\"cut".to_owned(),
    ]);
    assert_eq!(ran(Buck2, Some(&mixed)), 1);
    let mixed = lines(&["garbage".to_owned(), bazel_summary(2, "PASSED")]);
    assert_eq!(ran(Bazel, Some(&mixed)), 2);
    // A kind that is not a build system reads nothing, and neither kind reads the other's file.
    assert_eq!(ran(Unspecified, Some(buck2_result("PASS").as_bytes())), 0);
    assert_eq!(ran(Bazel, Some(buck2_result("PASS").as_bytes())), 0);
    assert_eq!(ran(Buck2, Some(bazel_summary(1, "PASSED").as_bytes())), 0);
}
