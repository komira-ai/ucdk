// SPDX-License-Identifier: Apache-2.0
//! Every row of the error table: what each answer of mopm, and each failure to get one, means to
//! the verb that sent the request.

mod common;

use std::time::Duration;

use common::{error_body, response, secs};
use ucdk_release::codec::GrantFault;
use ucdk_release_mopm::{
    ConflictAnswer, ConflictReason, DEFAULT_RETRY_AFTER_SECONDS, Mapped, Op, Response,
    TransportError, error_code, error_reason, map_reply, map_schema_violation, map_transport,
};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Backoff, SeamError};

const CEILING: Duration = Duration::from_secs(600);

fn answer(status: u16, code: &str) -> Response {
    response(status, &[], &error_body(code))
}

fn map(op: Op, response: &Response) -> Mapped {
    map_reply(op, response, CEILING).unwrap()
}

fn refused() -> Mapped {
    Mapped::Failed(SeamError::Refused(Reason::Channel))
}

fn denied() -> Mapped {
    Mapped::Failed(SeamError::GrantRefused(GrantFault::Denied))
}

fn expired() -> Mapped {
    Mapped::Failed(SeamError::GrantRefused(GrantFault::Expired))
}

fn ticks(n: u32) -> Mapped {
    Mapped::Failed(SeamError::RateLimited(Backoff { ticks: n }))
}

fn kind(mapped: &Mapped) -> &'static str {
    match mapped {
        Mapped::Missing => "missing",
        Mapped::Duplicate => "duplicate",
        Mapped::CommitRefused => "commit_refused",
        Mapped::Conflict(_) => "conflict",
        Mapped::Failed(SeamError::Refused(_)) => "refused",
        Mapped::Failed(SeamError::Permanent(_)) => "permanent",
        Mapped::Failed(SeamError::Transient(_)) => "transient",
        Mapped::Failed(SeamError::RateLimited(_)) => "rate_limited",
        Mapped::Failed(SeamError::InFlight(_)) => "in_flight",
        Mapped::Failed(SeamError::Unknown(_)) => "unknown",
        Mapped::Failed(SeamError::GrantRefused(_)) => "grant_refused",
    }
}

/// Checks `(status, code) -> kind` for every row of a table, for one op.
fn rows(op: Op, table: &[(u16, &str, &str)]) {
    for (status, code, expected) in table {
        let got = map(op, &answer(*status, code));
        assert_eq!(kind(&got), *expected, "{op:?} {status} {code}: {got:?}");
    }
}

#[test]
fn a_success_status_is_not_an_error() {
    for status in [200, 201, 202, 204, 299] {
        for op in [
            Op::Read,
            Op::Create,
            Op::Commit,
            Op::Poll,
            Op::Yank,
            Op::Abort,
        ] {
            assert_eq!(map_reply(op, &response(status, &[], ""), CEILING), None);
        }
    }
}

#[test]
fn reads_map_by_the_parsed_code() {
    assert_eq!(
        map(Op::Read, &answer(404, "package_not_found")),
        Mapped::Missing
    );
    assert_eq!(
        map(Op::Read, &answer(404, "version_not_found")),
        Mapped::Missing
    );
    assert_eq!(map(Op::Read, &answer(404, "route_not_found")), refused());
    assert_eq!(map(Op::Read, &answer(503, "read_only_mode")), refused());
    rows(
        Op::Read,
        &[
            // Any other 404 is a mis-pointed registry or a gateway, never an empty one.
            (404, "something_else", "transient"),
            (404, "not_in_index", "transient"),
            (502, "index_unavailable", "transient"),
            (500, "internal_error", "transient"),
            (503, "unavailable", "transient"),
            (400, "invalid_name", "transient"),
            (409, "version_exists", "transient"),
            (302, "redirect", "transient"),
        ],
    );
}

#[test]
fn a_read_failure_whose_body_cannot_be_read_is_never_absent() {
    let bodies = [
        "",
        "not json",
        "<html>Not Found</html>",
        "{}",
        r#"{"error":{}}"#,
        r#"{"error":{"code":5}}"#,
        r#"{"error":"package_not_found"}"#,
        r#"{"code":"package_not_found"}"#,
        "[]",
    ];
    for body in bodies {
        let got = map(Op::Read, &response(404, &[], body));
        assert_eq!(kind(&got), "transient", "{body:?}: {got:?}");
    }
    assert_eq!(error_code(error_body("x").as_bytes()), Some("x".to_owned()));
    assert_eq!(error_code(b""), None);
    assert_eq!(error_code(br#"{"error":{"code":1}}"#), None);
}

#[test]
fn authorisation_failures_before_a_commit_are_grant_refusals() {
    for op in [Op::Read, Op::Create, Op::Yank] {
        for code in ["token_unknown", "token_revoked", "token_missing"] {
            assert_eq!(map(op, &answer(401, code)), denied(), "{op:?} {code}");
        }
        assert_eq!(map(op, &answer(403, "forbidden")), denied(), "{op:?}");
        assert_eq!(map(op, &answer(401, "token_expired")), expired(), "{op:?}");
        // Only a 401 is an expiry; the same code under another status is a refusal.
        assert_eq!(map(op, &answer(403, "token_expired")), denied(), "{op:?}");
    }
}

#[test]
fn create_maps_every_row() {
    rows(
        Op::Create,
        &[
            (404, "route_not_found", "refused"),
            (503, "read_only_mode", "refused"),
            (404, "package_not_found", "refused"),
            (403, "not_owner", "refused"),
            (403, "limit_exceeded", "refused"),
            (409, "limit_exceeded", "refused"),
            (413, "payload_too_large", "refused"),
            (409, "version_exists", "duplicate"),
            (409, "group_conflict", "conflict"),
            (400, "invalid_name", "permanent"),
            (400, "invalid_version", "permanent"),
            (400, "invalid_request", "permanent"),
            (400, "upload_invalid", "permanent"),
            (404, "version_not_found", "permanent"),
            (404, "something_else", "permanent"),
            (503, "unavailable", "transient"),
            (500, "internal_error", "transient"),
            (502, "bad_gateway", "transient"),
        ],
    );
    let bare = map(Op::Create, &response(404, &[], "<html>"));
    assert_eq!(kind(&bare), "permanent");
}

fn conflict(reason: Option<&str>) -> Response {
    let reason = reason.map_or(String::new(), |r| format!(r#","reason":{r}"#));
    response(
        409,
        &[],
        &format!(r#"{{"error":{{"code":"group_conflict","message":"m","hint":"h"{reason}}}}}"#),
    )
}

#[test]
fn a_group_conflict_is_classified_by_its_reason_field_alone() {
    let listed = [
        ("number_held", ConflictReason::NumberHeld),
        ("group_failed", ConflictReason::GroupFailed),
        ("open_cap", ConflictReason::OpenCap),
        ("supersede_cap", ConflictReason::SupersedeCap),
        ("takeover_blocked", ConflictReason::TakeoverBlocked),
        ("number_taken", ConflictReason::NumberTaken),
        ("overlap_refused", ConflictReason::OverlapRefused),
    ];
    for (text, reason) in listed {
        let got = map(Op::Create, &conflict(Some(&format!("\"{text}\""))));
        assert_eq!(got, Mapped::Conflict(reason), "{text}");
        assert_eq!(ConflictReason::parse(Some(text)), reason);
    }
    // A registry that predates the field, a reason not on the list, a value that is not text,
    // and a reason-like word in the message are all an absent reason.
    let unlisted = [
        conflict(None),
        conflict(Some("\"a_new_reason\"")),
        conflict(Some("\"NUMBER_HELD\"")),
        conflict(Some("\"\"")),
        conflict(Some("5")),
        conflict(Some("null")),
        conflict(Some(r#"["number_held"]"#)),
        response(
            409,
            &[],
            r#"{"error":{"code":"group_conflict","message":"number_held open_cap","hint":"group_failed"}}"#,
        ),
    ];
    for response in &unlisted {
        assert_eq!(
            map(Op::Create, response),
            Mapped::Conflict(ConflictReason::Unlisted),
            "{response:?}"
        );
    }
    assert_eq!(ConflictReason::parse(None), ConflictReason::Unlisted);
    // The reason belongs to a conflict on create: on any other code it is not read.
    let other = response(
        409,
        &[],
        r#"{"error":{"code":"version_exists","reason":"number_held"}}"#,
    );
    assert_eq!(map(Op::Create, &other), Mapped::Duplicate);
}

#[test]
fn the_reason_is_read_from_the_error_object_only() {
    let body = br#"{"error":{"code":"group_conflict","reason":"open_cap"}}"#;
    assert_eq!(error_reason(body).as_deref(), Some("open_cap"));
    for body in [
        &b""[..],
        b"not json",
        br#"{"reason":"open_cap"}"#,
        br#"{"error":{"reason":7}}"#,
        br#"{"error":"open_cap"}"#,
    ] {
        assert_eq!(error_reason(body), None, "{body:?}");
    }
}

#[test]
fn each_reason_has_one_class_and_a_cap_is_never_a_duplicate() {
    let refused = ConflictAnswer::Fail(SeamError::Refused(Reason::Channel));
    assert_eq!(
        ConflictReason::NumberHeld.answer(),
        ConflictAnswer::Duplicate
    );
    assert_eq!(
        ConflictReason::OverlapRefused.answer(),
        ConflictAnswer::Duplicate
    );
    assert_eq!(
        ConflictReason::NumberTaken.answer(),
        ConflictAnswer::DuplicateIfHeld
    );
    assert_eq!(ConflictReason::OpenCap.answer(), refused);
    assert_eq!(ConflictReason::SupersedeCap.answer(), refused);
    // A failed group's burn is an effect: transient, never permanent and never a duplicate.
    for reason in [ConflictReason::GroupFailed, ConflictReason::TakeoverBlocked] {
        assert!(
            matches!(
                reason.answer(),
                ConflictAnswer::Fail(SeamError::Transient(_))
            ),
            "{reason:?}"
        );
    }
    // Nothing says what an unknown reason means: the engine goes to its read-back.
    assert!(matches!(
        ConflictReason::Unlisted.answer(),
        ConflictAnswer::Fail(SeamError::Unknown(_))
    ));
}

#[test]
fn commit_is_unknown_unless_mopm_refused_it_outright() {
    rows(
        Op::Commit,
        &[
            (409, "group_commit_refused", "commit_refused"),
            (400, "upload_invalid", "transient"),
            (503, "read_only_mode", "refused"),
            (404, "route_not_found", "refused"),
            (401, "token_expired", "unknown"),
            (401, "token_unknown", "unknown"),
            (403, "forbidden", "unknown"),
            (404, "group_not_found", "unknown"),
            (409, "version_exists", "unknown"),
            (500, "internal_error", "unknown"),
            (503, "unavailable", "unknown"),
            (302, "redirect", "unknown"),
        ],
    );
    let bare = map(Op::Commit, &response(502, &[], ""));
    let expected = SeamError::Unknown("mopm answered 502 with no error code".to_owned());
    assert_eq!(bare, Mapped::Failed(expected));
}

#[test]
fn a_poll_after_the_commit_is_unknown_whatever_it_answers() {
    rows(
        Op::Poll,
        &[
            (401, "token_expired", "unknown"),
            (401, "token_unknown", "unknown"),
            (404, "group_not_found", "unknown"),
            (404, "route_not_found", "unknown"),
            (429, "rate_limited", "unknown"),
            (500, "internal_error", "unknown"),
            (503, "read_only_mode", "unknown"),
            (503, "unavailable", "unknown"),
        ],
    );
}

#[test]
fn yank_maps_every_row() {
    rows(
        Op::Yank,
        &[
            (404, "package_not_found", "refused"),
            (403, "not_owner", "refused"),
            (404, "route_not_found", "refused"),
            (409, "invalid_transition", "permanent"),
            (400, "invalid_request", "permanent"),
            (404, "version_not_found", "permanent"),
            (503, "unavailable", "unknown"),
            (500, "internal_error", "unknown"),
            (400, "something_else", "unknown"),
        ],
    );
    assert_eq!(map(Op::Yank, &answer(401, "token_expired")), expired());
    assert_eq!(map(Op::Yank, &answer(401, "token_revoked")), denied());
}

#[test]
fn abort_maps_every_row() {
    rows(
        Op::Abort,
        &[
            (404, "route_not_found", "refused"),
            (403, "forbidden", "grant_refused"),
            (401, "token_unknown", "grant_refused"),
            (404, "group_not_found", "unknown"),
            (409, "group_not_open", "unknown"),
            (503, "unavailable", "unknown"),
            (500, "internal_error", "unknown"),
        ],
    );
    assert_eq!(map(Op::Abort, &answer(401, "token_expired")), expired());
    assert_eq!(map(Op::Abort, &answer(429, "rate_limited")), ticks(5));
}

#[test]
fn a_429_is_rate_limited_for_the_ticks_retry_after_states() {
    let limited =
        |retry: &str| response(429, &[("Retry-After", retry)], &error_body("rate_limited"));
    for op in [Op::Read, Op::Create, Op::Commit, Op::Yank] {
        assert_eq!(map(op, &limited("7")), ticks(7), "{op:?}");
    }
    assert_eq!(map(Op::Create, &limited(" 30 ")), ticks(30));
    // A wait of nothing is still one tick: never a retry storm.
    assert_eq!(map(Op::Create, &limited("0")), ticks(1));
    // The header name is not case sensitive.
    let lower = response(429, &[("retry-after", "3")], "");
    assert_eq!(map(Op::Create, &lower), ticks(3));
}

#[test]
fn a_429_without_a_usable_retry_after_waits_a_fixed_small_default() {
    let default = ticks(u32::try_from(DEFAULT_RETRY_AFTER_SECONDS).unwrap());
    let with = |headers: &[(&str, &str)]| response(429, headers, &error_body("rate_limited"));
    assert_eq!(map(Op::Create, &with(&[])), default);
    let unusable = [
        "",
        "   ",
        "Wed, 21 Oct 2026 07:28:00 GMT",
        "-5",
        "1.5",
        "+5",
        "5s",
        "\u{663}",
    ];
    for value in unusable {
        let got = map(Op::Create, &with(&[("retry-after", value)]));
        assert_eq!(got, default, "{value:?}");
    }
}

#[test]
fn the_retry_after_ceiling_is_pinned_on_both_sides() {
    let limited = |retry: &str| response(429, &[("retry-after", retry)], "");
    let create =
        |retry: &str, ceiling: Duration| map_reply(Op::Create, &limited(retry), ceiling).unwrap();
    assert_eq!(create("600", secs(600)), ticks(600));
    assert_eq!(create("601", secs(600)), refused());
    // The seconds to the next midnight, as the daily file allowance states them.
    assert_eq!(create("86400", secs(600)), refused());
    assert_eq!(create("86400", secs(86_400)), ticks(86_400));
    // A number too large for 64 bits is over every ceiling.
    assert_eq!(
        create("99999999999999999999999999", secs(u64::MAX - 1)),
        refused()
    );
    // The default wait is held to the ceiling too.
    let bare = response(429, &[], "");
    let below = map_reply(Op::Create, &bare, secs(DEFAULT_RETRY_AFTER_SECONDS - 1));
    assert_eq!(below, Some(refused()));
    let at = map_reply(Op::Create, &bare, secs(DEFAULT_RETRY_AFTER_SECONDS));
    assert_eq!(kind(&at.unwrap()), "rate_limited");
}

#[test]
fn a_transport_failure_is_unknown_where_the_request_may_have_landed() {
    for error in [
        TransportError::never_connected("refused"),
        TransportError::no_usable_answer("timed out"),
    ] {
        for op in [Op::Commit, Op::Poll, Op::Yank, Op::Abort] {
            assert!(
                matches!(map_transport(op, &error), SeamError::Unknown(_)),
                "{op:?}"
            );
        }
        for op in [Op::Read, Op::Create] {
            assert!(
                matches!(map_transport(op, &error), SeamError::Transient(_)),
                "{op:?}"
            );
        }
    }
}

#[test]
fn a_well_formed_answer_that_breaks_the_schema_is_permanent_only_where_nothing_landed() {
    for op in [Op::Read, Op::Create] {
        let got = map_schema_violation(op, "a file without a sha256");
        assert!(
            matches!(&got, SeamError::Permanent(m) if m.contains("sha256")),
            "{got:?}"
        );
    }
    for op in [Op::Commit, Op::Poll, Op::Yank, Op::Abort] {
        let got = map_schema_violation(op, "no status");
        assert!(
            matches!(&got, SeamError::Unknown(m) if m.contains("no status")),
            "{got:?}"
        );
    }
}

#[test]
fn nothing_but_the_status_and_the_code_reaches_an_error() {
    let canary = "CANARY-TOKEN-0123";
    let body = format!(
        r#"{{"error":{{"code":"group_conflict","message":"{canary}","hint":"{canary}","reason":"{canary}"}}}}"#
    );
    for op in [
        Op::Read,
        Op::Create,
        Op::Commit,
        Op::Poll,
        Op::Yank,
        Op::Abort,
    ] {
        for status in [400, 401, 404, 409, 500, 503] {
            let mapped = map_reply(op, &response(status, &[], &body), CEILING).unwrap();
            assert!(!format!("{mapped:?}").contains(canary), "{op:?} {status}");
            if let Mapped::Failed(e) = &mapped {
                assert!(!e.to_string().contains(canary));
            }
            if let Mapped::Conflict(reason) = &mapped {
                let reason = *reason;
                assert_eq!(reason, ConflictReason::Unlisted);
                assert!(!format!("{:?}", reason.answer()).contains(canary));
            }
        }
    }
}
