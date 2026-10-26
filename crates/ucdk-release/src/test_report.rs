// SPDX-License-Identifier: Apache-2.0
//! Reading the report a test invocation wrote: how many tests ran, which is what makes a pass.
//! Both kinds write JSON lines; one line that is not JSON is not counted, and a report with
//! nothing counted, or no report, is zero.
//!
//! - Buck2 (`events.json-lines`): `ran` counts the lines that carry a `TestResult` record,
//!   anywhere in the event, whose `status` is a name other than `SKIP`, `OMITTED`,
//!   `LISTING_SUCCESS` and `LISTING_FAILED` (a listing is not a run). A record with no status
//!   name is not counted.
//! - Bazel (`events.json`, the build event protocol's JSON form): `ran` is the sum of
//!   `totalRunCount` over the lines whose `testSummary` has an `overallStatus` other than
//!   `NO_STATUS`, `FAILED_TO_BUILD` and `TOOL_HALTED_BEFORE_TESTING` (nothing was tested). A
//!   summary with no `overallStatus` says `NO_STATUS`, which JSON leaves out as a default, and
//!   is not counted. A report is the one invocation's, so every summary in it is the target's.
//!   This counts runs of the target and not cases.
//!
//! Nothing here reads console text, and the per-case result files are not read.

use serde_json::Value;
use ucdk_release_proto::v1::BuildSystemKind;

/// Buck2 statuses of a test result that is not a test having run.
const BUCK2_NOT_RUN: [&str; 4] = ["SKIP", "OMITTED", "LISTING_SUCCESS", "LISTING_FAILED"];

/// Bazel overall statuses of a summary that tested nothing.
const BAZEL_NOT_TESTED: [&str; 3] = ["NO_STATUS", "FAILED_TO_BUILD", "TOOL_HALTED_BEFORE_TESTING"];

/// How many tests the report of an invocation says ran. `None` is a report that could not be
/// read, and a kind that is not a build system reads nothing: both are zero.
pub fn ran(kind: BuildSystemKind, report: Option<&[u8]>) -> u32 {
    let Some(report) = report else {
        return 0;
    };
    let count: fn(&Value) -> u64 = match kind {
        BuildSystemKind::Buck2 => buck2_line,
        BuildSystemKind::Bazel => bazel_line,
        BuildSystemKind::Unspecified => return 0,
    };
    let total = report
        .split(|b| *b == b'\n')
        .filter_map(|line| serde_json::from_slice::<Value>(line).ok())
        .fold(0u64, |sum, event| sum.saturating_add(count(&event)));
    u32::try_from(total).unwrap_or(u32::MAX)
}

/// One when the event carries a test result that ran.
fn buck2_line(event: &Value) -> u64 {
    u64::from(has_run_result(event))
}

fn has_run_result(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, inner)| {
            (key == "TestResult" && ran_status(inner)) || has_run_result(inner)
        }),
        Value::Array(items) => items.iter().any(has_run_result),
        _ => false,
    }
}

fn ran_status(result: &Value) -> bool {
    result
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| !BUCK2_NOT_RUN.contains(&status))
}

/// The runs of the test summary of the event, when it has one that tested something.
fn bazel_line(event: &Value) -> u64 {
    let Some(summary) = event.get("testSummary") else {
        return 0;
    };
    let tested = summary
        .get("overallStatus")
        .and_then(Value::as_str)
        .is_some_and(|status| !BAZEL_NOT_TESTED.contains(&status));
    if !tested {
        return 0;
    }
    summary
        .get("totalRunCount")
        .and_then(Value::as_u64)
        .unwrap_or(0)
}
