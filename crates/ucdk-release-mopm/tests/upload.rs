// SPDX-License-Identifier: Apache-2.0
//! The uploads of `stage` against a registry written by hand: what is put and with which headers,
//! how storage answers are classified, when the upload URLs are renewed, and how a repeated stage
//! resumes a group.

mod common;

use common::*;
use serde_json::{Value, json};
use ucdk_core::ManualClock;
use ucdk_release_mopm::{Config, Member, Method, Registry, SetFile, Subdir, TransportError};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Backoff, SeamError, StageAnswer};

const OTHER: &str = "grp_57794d9daef2aedabe009f48791f7f3a";
const START: u64 = 1_000_000;

/// The proposed configuration with a stage budget so wide that it never ends a test.
fn wide() -> Config {
    let mut c = group_config();
    c.upload_budget = secs(3000);
    c
}

fn no_group(id: &str) -> Fake {
    Fake::default().on(
        Method::Get,
        &group_path(id),
        vec![error_reply(404, "group_not_found")],
    )
}

fn created(id: &str, members: &[Member], stored: &[&str]) -> Reply {
    json_reply(201, &created_json(id, "open", members, stored))
}

/// The second create answer of a refresh: other query and other header on every file.
fn fresh_json(id: &str, members: &[Member]) -> Value {
    let mut value = created_json(id, "open", members, &[]);
    for member in value["members"].as_array_mut().unwrap() {
        for file in member["files"].as_array_mut().unwrap() {
            let url = format!(
                "https://files.example.com/store/{}/{}?X-Amz-Signature=FRESH",
                file["subdir"].as_str().unwrap(),
                file["filename"].as_str().unwrap()
            );
            file["put_url"] = json!(url);
            file["put_headers"] = json!({"x-amz-checksum-sha256": "def"});
        }
    }
    value
}

fn stage_with(
    fake: Fake,
    config: Config,
    clock: &ManualClock,
) -> (Result<StageAnswer, SeamError>, Registry<Fake>) {
    let (owned, members, _) = world();
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let mut reg = declared_registry(fake, config, clock);
    let result = reg.stage(&caller(), &members, &files);
    (result, reg)
}

fn stage_on(fake: Fake) -> (Result<StageAnswer, SeamError>, Registry<Fake>) {
    stage_with(fake, wide(), &ManualClock::at(START))
}

fn lin() -> String {
    format!("PUT {}", put_path("linux-64"))
}

fn noa() -> String {
    format!("PUT {}", put_path("noarch"))
}

fn base_lines(id: &str) -> Vec<String> {
    vec![format!("GET {}", group_path(id)), format!("POST {GROUPS}")]
}

fn with_puts(id: &str, rest: &[String]) -> Vec<String> {
    let mut all = base_lines(id);
    all.extend(rest.iter().cloned());
    all
}

fn retry_after(seconds: Option<&str>) -> Reply {
    let headers: Vec<(&str, &str)> = seconds.map(|s| ("retry-after", s)).into_iter().collect();
    Ok(response(429, &headers, ""))
}

// ---- storage 429 ----

#[test]
fn storage_429_with_retry_after_is_rate_limited_by_it_and_is_not_retried() {
    let (_, members, id) = world();
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
        .on(
            Method::Put,
            &put_path("linux-64"),
            vec![retry_after(Some("7"))],
        )
        .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
    let (result, reg) = stage_on(fake);
    assert_eq!(result, Err(SeamError::RateLimited(Backoff { ticks: 7 })));
    assert_eq!(reg.transport().lines(), with_puts(&id, &[lin()]));
}

#[test]
fn storage_429_without_a_usable_retry_after_waits_the_default_of_five_seconds() {
    let (_, members, id) = world();
    for header in [None, Some("soon"), Some("")] {
        let fake = no_group(&id)
            .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
            .on(
                Method::Put,
                &put_path("linux-64"),
                vec![retry_after(header)],
            )
            .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
        let (result, reg) = stage_on(fake);
        assert_eq!(
            result,
            Err(SeamError::RateLimited(Backoff::after_seconds(5))),
            "{header:?}"
        );
        assert_eq!(reg.transport().lines(), with_puts(&id, &[lin()]));
    }
}

#[test]
fn storage_429_with_retry_after_at_the_ceiling_waits_and_above_it_is_refused() {
    let (_, members, id) = world();
    let cases = [
        (
            "600",
            Err(SeamError::RateLimited(Backoff::after_seconds(600))),
        ),
        ("601", Err(SeamError::Refused(Reason::Channel))),
    ];
    for (header, expected) in cases {
        let fake = no_group(&id)
            .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
            .on(
                Method::Put,
                &put_path("linux-64"),
                vec![retry_after(Some(header))],
            )
            .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
        let (result, reg) = stage_on(fake);
        assert_eq!(result, expected, "{header}");
        assert_eq!(reg.transport().lines(), with_puts(&id, &[lin()]));
    }
}

// ---- the age of the upload URLs ----

fn aged_by(bump_ms: u64) -> Vec<String> {
    let (_, members, id) = world();
    let clock = ManualClock::at(START);
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(200)])
        .bump(
            Method::Put,
            &put_path("linux-64"),
            &clock,
            std::time::Duration::from_millis(bump_ms),
        );
    let (result, reg) = stage_with(fake, wide(), &clock);
    assert_eq!(result, Ok(StageAnswer::Staged));
    reg.transport().lines()
}

#[test]
fn a_put_starting_149_999_ms_after_the_create_answer_uses_that_answer() {
    let (_, _, id) = world();
    assert_eq!(aged_by(149_999), with_puts(&id, &[lin(), noa()]));
}

#[test]
fn a_put_starting_150_000_ms_after_the_create_answer_is_preceded_by_a_refresh() {
    let (_, _, id) = world();
    let expected = with_puts(&id, &[lin(), format!("POST {GROUPS}"), noa()]);
    assert_eq!(aged_by(150_000), expected);
}

#[test]
fn a_put_starting_150_001_ms_after_the_create_answer_is_preceded_by_a_refresh() {
    let (_, _, id) = world();
    let expected = with_puts(&id, &[lin(), format!("POST {GROUPS}"), noa()]);
    assert_eq!(aged_by(150_001), expected);
}

#[test]
fn a_retry_that_starts_after_the_boundary_is_preceded_by_a_refresh() {
    let (_, members, id) = world();
    let clock = ManualClock::at(START);
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(503), ok(200)])
        .bump(
            Method::Put,
            &put_path("noarch"),
            &clock,
            std::time::Duration::from_millis(150_000),
        );
    let (result, reg) = stage_with(fake, wide(), &clock);
    assert_eq!(result, Ok(StageAnswer::Staged));
    let expected = with_puts(&id, &[lin(), noa(), format!("POST {GROUPS}"), noa()]);
    assert_eq!(reg.transport().lines(), expected);
}

// ---- upload headers ----

#[test]
fn the_uploads_carry_exactly_the_headers_mopm_gave_and_never_the_token() {
    let (_, members, id) = world();
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
    let (result, reg) = stage_on(fake);
    assert_eq!(result, Ok(StageAnswer::Staged));
    let log = &reg.transport().log;
    assert_eq!(log.len(), 4);
    let header = |n: &str, v: &str| (n.to_owned(), v.to_owned());
    assert_eq!(
        log[0].headers,
        [
            header("accept", "application/json"),
            header("authorization", "<secret>"),
        ]
    );
    assert_eq!(
        log[1].headers,
        [
            header("accept", "application/json"),
            header("authorization", "<secret>"),
            header("content-type", "application/json"),
        ]
    );
    let token = String::from_utf8_lossy(TOKEN).into_owned();
    for put in &log[2..] {
        assert_eq!(
            put.headers,
            [("x-amz-checksum-sha256".to_owned(), "abc".to_owned())]
        );
        assert_eq!(put.bearer, None);
        assert!(
            put.headers
                .iter()
                .all(|(n, v)| !n.contains(&token) && !v.contains(&token))
        );
    }
    for logged in log {
        assert!(logged.headers.iter().all(|(n, _)| {
            !(logged.method == Method::Put && n.eq_ignore_ascii_case("authorization"))
        }));
    }
}

#[test]
fn a_header_that_holds_the_callers_credential_is_refused_and_nothing_is_put() {
    let (_, members, id) = world();
    let token = String::from_utf8_lossy(TOKEN).into_owned();
    // The credential as a value, whole or after its scheme, and as a header name.
    let echoes = [
        json!({"x-echo": format!("Bearer {token}")}),
        json!({"x-echo": token.clone()}),
        json!({ token: "1" }),
    ];
    for echo in echoes {
        let mut body = created_json(&id, "open", &members, &[]);
        body["members"][0]["files"][0]["put_headers"] = echo;
        let fake = no_group(&id).on(Method::Post, GROUPS, vec![json_reply(201, &body)]);
        let (result, reg) = stage_on(fake);
        assert_eq!(
            result,
            Err(SeamError::Permanent(
                "the upload headers mopm gave hold the caller's credential; nothing was put"
                    .to_owned()
            ))
        );
        assert_eq!(reg.transport().lines(), base_lines(&id));
    }
}

#[test]
fn a_create_answer_with_an_unusable_upload_header_is_malformed_and_nothing_is_put() {
    let (_, members, id) = world();
    let cases = [
        json!({"Authorization": "x"}),
        json!({"AUTHORIZATION": "x"}),
        json!({"proxy-authorization": "x"}),
        json!({"Proxy-Authorization": "x"}),
        json!({"bad name": "x"}),
        json!({"bad:name": "x"}),
        json!({"": "x"}),
        json!({"x-a": "line\nbreak"}),
        json!({"x-a": "caf\u{e9}"}),
        json!({"x-a": 5}),
        json!({"x-a": "del\u{7f}"}),
        json!({"X-A": "1", "x-a": "2"}),
        json!(["x-a", "1"]),
    ];
    for headers in cases {
        let mut body = created_json(&id, "open", &members, &[]);
        body["members"][0]["files"][0]["put_headers"] = headers.clone();
        let fake = no_group(&id).on(Method::Post, GROUPS, vec![json_reply(201, &body)]);
        let (result, reg) = stage_on(fake);
        assert_eq!(
            result,
            Err(SeamError::Permanent(
                "malformed answer: a group create".to_owned()
            )),
            "{headers}"
        );
        assert_eq!(reg.transport().lines(), base_lines(&id), "{headers}");
    }
}

// ---- what is sent ----

#[test]
fn the_create_body_declares_the_set_exactly_and_refuses_overlap() {
    let (_, members, id) = world();
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
    let (_, reg) = stage_on(fake);
    let file = file_name("alpha", "1.0.0");
    let sent: Value = serde_json::from_slice(&reg.transport().log[1].raw).unwrap();
    assert_eq!(
        sent,
        json!({
            "members": [{
                "name": "alpha",
                "version": "1.0.0",
                "files": [
                    {"subdir": "linux-64", "filename": file, "sha256": DIGEST, "size": 1000},
                    {"subdir": "noarch", "filename": file, "sha256": DIGEST, "size": 1000},
                ],
            }],
            "on_overlap": "refuse",
        })
    );
}

#[test]
fn each_file_is_put_with_its_own_bytes() {
    let (owned, members, id) = world();
    let linux: Vec<u8> = vec![1u8; 1000];
    let noarch: Vec<u8> = (0..1000).map(|i| (i % 251) as u8).collect();
    let name = &owned[0].coordinate.name;
    let version = &owned[0].coordinate.version;
    let files = [
        SetFile {
            name,
            version,
            subdir: Subdir::of_platform("linux-x86_64").unwrap(),
            bytes: &linux,
        },
        SetFile {
            name,
            version,
            subdir: Subdir::of_platform("any").unwrap(),
            bytes: &noarch,
        },
    ];
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
    let mut reg = declared_registry(fake, wide(), &ManualClock::at(START));
    assert_eq!(
        reg.stage(&caller(), &members, &files),
        Ok(StageAnswer::Staged)
    );
    let log = &reg.transport().log;
    assert_eq!(log[2].path, put_path("linux-64"));
    assert_eq!(log[2].raw, linux);
    assert_eq!(log[3].path, put_path("noarch"));
    assert_eq!(log[3].raw, noarch);
    assert_ne!(log[2].raw, log[3].raw);
}

// ---- refreshes ----

/// The two-file set where linux-64 is put and noarch is refused with 403 once, then answers
/// `then`; the first create is plain and the refresh answers `second`.
fn refreshed(second: Reply, then: Reply, id: &str, members: &[Member]) -> Fake {
    no_group(id)
        .on(
            Method::Post,
            GROUPS,
            vec![created(id, members, &[]), second],
        )
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(403), then])
}

#[test]
fn a_refresh_that_gives_a_new_query_and_header_is_used_by_the_retried_put() {
    let (_, members, id) = world();
    let second = json_reply(201, &fresh_json(&id, &members));
    let (result, reg) = stage_on(refreshed(second, ok(200), &id, &members));
    assert_eq!(result, Ok(StageAnswer::Staged));
    let expected = with_puts(&id, &[lin(), noa(), format!("POST {GROUPS}"), noa()]);
    assert_eq!(reg.transport().lines(), expected);
    let log = &reg.transport().log;
    let sig = |l: &Logged| l.query_text.clone();
    let old = Some("X-Amz-Signature=CANARYSIG".to_owned());
    let new = Some("X-Amz-Signature=FRESH".to_owned());
    assert_eq!((sig(&log[2]), sig(&log[3])), (old.clone(), old));
    assert_eq!(sig(&log[5]), new);
    let abc = vec![("x-amz-checksum-sha256".to_owned(), "abc".to_owned())];
    let def = vec![("x-amz-checksum-sha256".to_owned(), "def".to_owned())];
    assert_eq!((&log[2].headers, &log[3].headers), (&abc, &abc));
    assert_eq!(log[5].headers, def);
    assert_eq!(log[5].raw, file_bytes());
}

#[test]
fn a_second_403_after_a_refresh_is_transient() {
    let (_, members, id) = world();
    let second = created(&id, &members, &[]);
    let (result, reg) = stage_on(refreshed(second, ok(403), &id, &members));
    assert_eq!(
        result,
        Err(SeamError::Transient(
            "an upload URL expired again".to_owned()
        ))
    );
    let expected = with_puts(&id, &[lin(), noa(), format!("POST {GROUPS}"), noa()]);
    assert_eq!(reg.transport().lines(), expected);
}

#[test]
fn a_refresh_the_registry_answers_with_500_is_transient_and_nothing_more_is_put() {
    let (_, members, id) = world();
    let (result, reg) = stage_on(refreshed(
        error_reply(500, "internal"),
        ok(200),
        &id,
        &members,
    ));
    assert_eq!(
        result,
        Err(SeamError::Transient(
            "mopm answered 500 internal".to_owned()
        ))
    );
    let expected = with_puts(&id, &[lin(), noa(), format!("POST {GROUPS}")]);
    assert_eq!(reg.transport().lines(), expected);
}

#[test]
fn a_refresh_whose_answer_is_lost_is_transient_and_nothing_more_is_put() {
    let (_, members, id) = world();
    let (result, reg) = stage_on(refreshed(lost(), ok(200), &id, &members));
    assert_eq!(
        result,
        Err(SeamError::Transient(
            TransportError::no_usable_answer("the answer was lost").to_string()
        ))
    );
    let expected = with_puts(&id, &[lin(), noa(), format!("POST {GROUPS}")]);
    assert_eq!(reg.transport().lines(), expected);
}

#[test]
fn a_refreshed_url_on_a_foreign_origin_is_transient_and_nothing_is_put_to_it() {
    let (_, members, id) = world();
    let mut body = created_json(&id, "open", &members, &[]);
    for file in body["members"][0]["files"].as_array_mut().unwrap() {
        file["put_url"] = json!("https://evil.example.net/foreign/x?sig=1");
    }
    let (result, reg) = stage_on(refreshed(json_reply(201, &body), ok(200), &id, &members));
    assert_eq!(
        result,
        Err(SeamError::Transient(
            "an upload URL names an origin that is not configured".to_owned()
        ))
    );
    let expected = with_puts(&id, &[lin(), noa(), format!("POST {GROUPS}")]);
    assert_eq!(reg.transport().lines(), expected);
}

#[test]
fn a_refresh_that_names_another_group_is_permanent_when_that_group_is_aborted() {
    let (_, members, id) = world();
    let second = created(OTHER, &members, &[]);
    let fake = refreshed(second, ok(200), &id, &members).on(
        Method::Delete,
        &group_path(OTHER),
        vec![ok(200)],
    );
    let (result, reg) = stage_on(fake);
    assert_eq!(
        result,
        Err(SeamError::Permanent(
            "the group id derivation drifted: the registry named another group than the one computed; the group it named was aborted"
                .to_owned()
        ))
    );
    let expected = with_puts(
        &id,
        &[
            lin(),
            noa(),
            format!("POST {GROUPS}"),
            format!("DELETE {}", group_path(OTHER)),
        ],
    );
    assert_eq!(reg.transport().lines(), expected);
}

#[test]
fn a_refresh_that_names_another_group_is_unknown_when_that_group_cannot_be_aborted() {
    let (_, members, id) = world();
    let second = created(OTHER, &members, &[]);
    let (result, reg) = stage_on(refreshed(second, ok(200), &id, &members));
    assert_eq!(
        result,
        Err(SeamError::Unknown(
            "the group id derivation drifted and the group the registry named could not be aborted"
                .to_owned()
        ))
    );
    assert_eq!(reg.transport().count(Method::Put, &put_path("noarch")), 1);
}

#[test]
fn a_refresh_that_lists_a_superseded_group_is_refused_and_nothing_more_is_put() {
    let (_, members, id) = world();
    let mut body = created_json(&id, "open", &members, &[]);
    body["superseded"] = json!([OTHER]);
    let (result, reg) = stage_on(refreshed(json_reply(201, &body), ok(200), &id, &members));
    assert_eq!(result, Err(SeamError::Refused(Reason::Channel)));
    let expected = with_puts(&id, &[lin(), noa(), format!("POST {GROUPS}")]);
    assert_eq!(reg.transport().lines(), expected);
}

#[test]
fn a_refresh_that_answers_the_group_past_open_after_bytes_were_sent_is_staged() {
    let (_, members, id) = world();
    let second = json_reply(200, &created_json(&id, "validating", &members, &[]));
    let (result, reg) = stage_on(refreshed(second, ok(200), &id, &members));
    assert_eq!(result, Ok(StageAnswer::Staged));
    let expected = with_puts(&id, &[lin(), noa(), format!("POST {GROUPS}")]);
    assert_eq!(reg.transport().lines(), expected);
}

// ---- retries ----

#[test]
fn a_put_that_keeps_failing_is_tried_three_times_then_transient() {
    let (_, members, id) = world();
    let replies: Vec<Reply> = vec![
        ok(500),
        ok(502),
        ok(503),
        ok(504),
        lost(),
        Err(TransportError::never_connected("x")),
    ];
    for reply in replies {
        let fake = no_group(&id)
            .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
            .on(Method::Put, &put_path("linux-64"), vec![reply.clone()])
            .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
        let (result, reg) = stage_on(fake);
        let last = match &reply {
            Ok(response) => format!("storage answered {}", response.status),
            Err(error) => error.to_string(),
        };
        assert_eq!(
            result,
            Err(SeamError::Transient(format!(
                "an upload failed after 3 tries: {last}"
            )))
        );
        assert_eq!(
            reg.transport().lines(),
            with_puts(&id, &[lin(), lin(), lin()])
        );
    }
}

#[test]
fn the_message_of_an_exhausted_put_names_the_last_failure() {
    let (_, members, id) = world();
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
        .on(Method::Put, &put_path("linux-64"), vec![ok(503)]);
    let (result, _) = stage_on(fake);
    assert_eq!(
        result,
        Err(SeamError::Transient(
            "an upload failed after 3 tries: storage answered 503".to_owned()
        ))
    );
}

#[test]
fn without_retries_a_failing_put_is_tried_once() {
    let (_, members, id) = world();
    let mut config = wide();
    config.retries = 0;
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
        .on(Method::Put, &put_path("linux-64"), vec![ok(503)]);
    let (result, reg) = stage_with(fake, config, &ManualClock::at(START));
    assert_eq!(
        result,
        Err(SeamError::Transient(
            "an upload failed after 1 tries: storage answered 503".to_owned()
        ))
    );
    assert_eq!(reg.transport().lines(), with_puts(&id, &[lin()]));
}

#[test]
fn another_storage_status_is_transient_and_not_retried() {
    let (_, members, id) = world();
    // 501 and 505 are server failures outside the retried four.
    for status in [400, 501, 505] {
        let fake = no_group(&id)
            .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
            .on(Method::Put, &put_path("linux-64"), vec![ok(status)]);
        let (result, reg) = stage_on(fake);
        assert_eq!(
            result,
            Err(SeamError::Transient(format!(
                "storage answered {status} to an upload"
            )))
        );
        assert_eq!(reg.transport().lines(), with_puts(&id, &[lin()]));
    }
}

#[test]
fn a_create_that_stays_reserving_with_no_repeats_is_sent_once_and_transient() {
    let (_, members, id) = world();
    let mut body = created_json(&id, "open", &members, &[]);
    body["reserving"] = json!({"done": 1, "total": 2});
    let mut config = wide();
    config.reserving_repeats = 0;
    let fake = no_group(&id).on(Method::Post, GROUPS, vec![json_reply(202, &body)]);
    let (result, reg) = stage_with(fake, config, &ManualClock::at(START));
    assert_eq!(
        result,
        Err(SeamError::Transient(
            "the registry is still reserving the set; the next stage resumes it".to_owned()
        ))
    );
    assert_eq!(reg.transport().lines(), base_lines(&id));
}

// ---- the life of a group across stages ----

#[test]
fn an_ended_group_is_reopened_by_a_create_and_both_files_are_put() {
    let (_, members, id) = world();
    for status in ["aborted", "expired"] {
        let ended = group_json(&id, status, &members, &[]);
        let fake = Fake::default()
            .on(Method::Get, &group_path(&id), vec![json_reply(200, &ended)])
            .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
            .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
            .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
        let (result, reg) = stage_on(fake);
        assert_eq!(result, Ok(StageAnswer::Staged), "{status}");
        assert_eq!(
            reg.transport().lines(),
            with_puts(&id, &[lin(), noa()]),
            "{status}"
        );
    }
}

#[test]
fn a_second_stage_after_a_failed_one_puts_only_the_file_not_yet_stored() {
    let (owned, members, id) = world();
    let partial = group_json(&id, "open", &members, &["linux-64"]);
    let fake = Fake::default()
        .on(
            Method::Get,
            &group_path(&id),
            vec![
                error_reply(404, "group_not_found"),
                json_reply(200, &partial),
            ],
        )
        .on(
            Method::Post,
            GROUPS,
            vec![
                created(&id, &members, &[]),
                created(&id, &members, &["linux-64"]),
            ],
        )
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(400), ok(200)]);
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let mut reg = declared_registry(fake, wide(), &ManualClock::at(START));
    assert_eq!(
        reg.stage(&caller(), &members, &files),
        Err(SeamError::Transient(
            "storage answered 400 to an upload".to_owned()
        ))
    );
    assert_eq!(
        reg.stage(&caller(), &members, &files),
        Ok(StageAnswer::Staged)
    );
    let get = format!("GET {}", group_path(&id));
    let post = format!("POST {GROUPS}");
    assert_eq!(
        reg.transport().lines(),
        [get.clone(), post.clone(), lin(), noa(), get, post, noa()]
    );
    assert_eq!(reg.transport().count(Method::Put, &put_path("linux-64")), 1);
}

#[test]
fn a_refresh_answer_with_a_credential_or_an_unusable_header_puts_nothing_more() {
    let (_, members, id) = world();
    let token = String::from_utf8_lossy(TOKEN).into_owned();
    let cases = [
        (
            json!({"x-echo": token}),
            "the upload headers mopm gave hold the caller's credential; nothing was put",
        ),
        (
            json!({"Authorization": "x"}),
            "malformed answer: a group create",
        ),
    ];
    for (headers, message) in cases {
        let mut refreshed = created_json(&id, "open", &members, &["linux-64"]);
        refreshed["members"][0]["files"][1]["put_headers"] = headers;
        let fake = no_group(&id)
            .on(
                Method::Post,
                GROUPS,
                vec![created(&id, &members, &[]), json_reply(200, &refreshed)],
            )
            .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
            .on(Method::Put, &put_path("noarch"), vec![ok(403)]);
        let (result, reg) = stage_on(fake);
        assert_eq!(result, Err(SeamError::Permanent(message.to_owned())));
        assert_eq!(
            reg.transport().lines(),
            with_puts(&id, &[lin(), noa(), format!("POST {GROUPS}")])
        );
    }
}

#[test]
fn the_retries_of_a_file_are_counted_across_a_refresh_for_age() {
    let (_, members, id) = world();
    let clock = ManualClock::at(START);
    // The first file takes 120 s; each failing attempt on the second takes 30 s, so its second
    // attempt finds the answer 150 s old and the URLs are renewed between its attempts.
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![created(&id, &members, &[]), created(&id, &members, &[])],
        )
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(503)])
        .bump(Method::Put, &put_path("linux-64"), &clock, secs(120))
        .bump(Method::Put, &put_path("noarch"), &clock, secs(30));
    let (result, reg) = stage_with(fake, wide(), &clock);
    // retries = 2: three attempts in all, one before the refresh and two after it.
    assert_eq!(
        result,
        Err(SeamError::Transient(
            "an upload failed after 3 tries: storage answered 503".to_owned()
        ))
    );
    assert_eq!(
        reg.transport().lines(),
        with_puts(&id, &[lin(), noa(), format!("POST {GROUPS}"), noa(), noa()])
    );
}

#[test]
fn an_expired_url_renews_the_urls_without_spending_the_retries_of_the_file() {
    let (_, members, id) = world();
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![created(&id, &members, &[]), created(&id, &members, &[])],
        )
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(
            Method::Put,
            &put_path("noarch"),
            vec![ok(503), ok(403), ok(503)],
        );
    let (result, reg) = stage_on(fake);
    // The 403 is the one renewal the design allows; the two 503s before and after it and the
    // third failure make the three tries of the file.
    assert_eq!(
        result,
        Err(SeamError::Transient(
            "an upload failed after 3 tries: storage answered 503".to_owned()
        ))
    );
    assert_eq!(
        reg.transport().lines(),
        with_puts(
            &id,
            &[lin(), noa(), noa(), format!("POST {GROUPS}"), noa(), noa()]
        )
    );
}
