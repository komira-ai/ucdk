// SPDX-License-Identifier: Apache-2.0
//! `stage` against a registry written by hand: the group read, the create, the uploads, and what
//! each answer of mopm makes of the call.

mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use ucdk_core::ManualClock;
use ucdk_release::codec::GrantFault;
use ucdk_release_mopm::{Caller, Member, Method, Registry, SetFile};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Backoff, SeamError, StageAnswer};

const OTHER: &str = "grp_57794d9daef2aedabe009f48791f7f3a";
const START: u64 = 1_000_000;

fn fresh() -> ManualClock {
    ManualClock::at(START)
}

fn no_group(id: &str) -> Fake {
    Fake::default().on(
        Method::Get,
        &group_path(id),
        vec![error_reply(404, "group_not_found")],
    )
}

/// The `PUT` routes of both files of `alpha`, each answering `replies`.
fn puts(fake: Fake, replies: &[Reply]) -> Fake {
    fake.on(Method::Put, &put_path("linux-64"), replies.to_vec())
        .on(Method::Put, &put_path("noarch"), replies.to_vec())
}

fn created(id: &str, members: &[Member], stored: &[&str]) -> Reply {
    json_reply(201, &created_json(id, "open", members, stored))
}

fn stage_of(
    fake: Fake,
    owned: &[Owned],
    members: &[Member],
) -> (Result<StageAnswer, SeamError>, Registry<Fake>) {
    let bytes = file_bytes();
    let files = set_files(owned, &bytes);
    let mut reg = registry(fake, &fresh());
    let result = reg.stage(&caller(), members, &files);
    (result, reg)
}

#[test]
fn a_new_set_is_created_and_each_file_is_put_with_its_headers_and_no_token() {
    let (owned, members, id) = world();
    let fake = puts(
        no_group(&id).on(Method::Post, GROUPS, vec![created(&id, &members, &[])]),
        &[ok(200)],
    );
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(result, Ok(StageAnswer::Staged));
    assert_eq!(
        reg.transport().lines(),
        [
            format!("GET {}", group_path(&id)),
            format!("POST {GROUPS}"),
            format!("PUT {}", put_path("linux-64")),
            format!("PUT {}", put_path("noarch")),
        ]
    );
    let log = &reg.transport().log;
    assert!(log[0].authorized && log[1].authorized);
    assert!(!log[2].authorized && !log[3].authorized);
    assert!(log[2].query && log[3].query);
    assert_eq!(log[2].body.len(), 1000);
    // The create declares the set, and it never supersedes.
    let sent: Value = serde_json::from_str(&log[1].body).unwrap();
    assert_eq!(sent["on_overlap"], "refuse");
    assert_eq!(sent["members"][0]["name"], "alpha");
    assert_eq!(sent["members"][0]["files"].as_array().unwrap().len(), 2);
    assert_eq!(sent["members"][0]["files"][0]["subdir"], "linux-64");
}

#[test]
fn a_repeat_of_an_open_group_with_every_file_stored_is_duplicate_and_sends_no_create() {
    let (owned, members, id) = world();
    let open = group_json(&id, "open", &members, &["linux-64", "noarch"]);
    let fake = Fake::default().on(Method::Get, &group_path(&id), vec![json_reply(200, &open)]);
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(result, Ok(StageAnswer::Duplicate));
    assert_eq!(
        reg.transport().lines(),
        [format!("GET {}", group_path(&id))]
    );
}

#[test]
fn a_repeat_after_a_partial_upload_puts_only_the_files_the_registry_does_not_hold() {
    let (owned, members, id) = world();
    let open = group_json(&id, "open", &members, &["linux-64"]);
    let fake = puts(
        Fake::default()
            .on(Method::Get, &group_path(&id), vec![json_reply(200, &open)])
            .on(
                Method::Post,
                GROUPS,
                vec![json_reply(
                    200,
                    &created_json(&id, "open", &members, &["linux-64"]),
                )],
            ),
        &[ok(200)],
    );
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(result, Ok(StageAnswer::Staged));
    let transport = reg.transport();
    assert_eq!(transport.count(Method::Put, &put_path("linux-64")), 0);
    assert_eq!(transport.count(Method::Put, &put_path("noarch")), 1);
}

#[test]
fn a_create_that_finds_every_file_held_or_the_group_past_open_uploads_nothing_and_is_duplicate() {
    let (owned, members, id) = world();
    let all = ["linux-64", "noarch"];
    let held = json_reply(200, &created_json(&id, "open", &members, &all));
    let (result, reg) = stage_of(
        no_group(&id).on(Method::Post, GROUPS, vec![held]),
        &owned,
        &members,
    );
    assert_eq!(result, Ok(StageAnswer::Duplicate));
    assert_eq!(reg.transport().lines().len(), 2);
    // A group that is validating, committed or visible lists no files.
    for status in ["validating", "committed", "visible"] {
        let past = json_reply(200, &created_json(&id, status, &members, &[]));
        let reads = group_json(&id, status, &members, &[]);
        let fake = Fake::default()
            .on(Method::Get, &group_path(&id), vec![json_reply(200, &reads)])
            .on(Method::Post, GROUPS, vec![past]);
        let (result, reg) = stage_of(fake, &owned, &members);
        assert_eq!(result, Ok(StageAnswer::Duplicate), "{status}");
        assert_eq!(reg.transport().lines().len(), 2, "{status}");
    }
}

#[test]
fn a_different_set_that_shares_a_member_is_refused_by_the_registry_and_ends_nothing() {
    let (_, _, id) = world();
    let mut changed = alpha();
    changed[0].sha256 = "f".repeat(64);
    let other = set(&changed);
    let other_id = ucdk_release_mopm::group_id(ACTOR, &other);
    assert_ne!(other_id, id);
    let fake = no_group(&other_id).on(
        Method::Post,
        GROUPS,
        vec![conflict_reply(Some("overlap_refused"))],
    );
    let bytes = file_bytes();
    let files = set_files(&changed, &bytes);
    let mut reg = registry(fake, &fresh());
    let result = reg.stage(&caller(), &other, &files);
    assert_eq!(result, Ok(StageAnswer::Duplicate));
    // Nothing was put, and nothing was deleted: the first set's group is left as it was.
    assert_eq!(reg.transport().lines().len(), 2);
    assert_eq!(reg.transport().count(Method::Delete, &group_path(&id)), 0);
}

#[test]
fn each_group_conflict_reason_has_its_class_and_an_unknown_reason_is_unknown() {
    let (owned, members, id) = world();
    let table = [
        (Some("number_held"), "duplicate"),
        (Some("overlap_refused"), "duplicate"),
        (Some("open_cap"), "refused"),
        (Some("supersede_cap"), "refused"),
        (Some("group_failed"), "transient"),
        (Some("takeover_blocked"), "transient"),
        (Some("a_reason_from_a_newer_registry"), "unknown"),
        (None, "unknown"),
    ];
    for (reason, expected) in table {
        let fake = no_group(&id).on(Method::Post, GROUPS, vec![conflict_reply(reason)]);
        let (result, reg) = stage_of(fake, &owned, &members);
        assert_eq!(stage_kind(&result), expected, "{reason:?}");
        // Nothing is read or put to decide a class that needs no record.
        assert_eq!(reg.transport().lines().len(), 2, "{reason:?}");
    }
}

#[test]
fn number_taken_is_duplicate_only_when_the_version_record_shows_every_coordinate_held() {
    let (owned, members, id) = world();
    let record = record_path("alpha", "1.0.0");
    let cases = [
        (
            json_reply(
                200,
                &record_json("alpha", "1.0.0", "published", &["linux-64", "noarch"]),
            ),
            "duplicate",
        ),
        (
            json_reply(
                200,
                &record_json("alpha", "1.0.0", "yanked", &["linux-64", "noarch"]),
            ),
            "duplicate",
        ),
        (
            json_reply(
                200,
                &record_json("alpha", "1.0.0", "published", &["linux-64"]),
            ),
            "transient",
        ),
        (
            json_reply(200, &record_json("alpha", "1.0.0", "tombstoned", &[])),
            "transient",
        ),
        (error_reply(404, "version_not_found"), "transient"),
        (error_reply(500, "internal_error"), "transient"),
    ];
    for (reply, expected) in cases {
        let fake = no_group(&id)
            .on(
                Method::Post,
                GROUPS,
                vec![conflict_reply(Some("number_taken"))],
            )
            .on(Method::Get, &record, vec![reply]);
        let (result, reg) = stage_of(fake, &owned, &members);
        assert_eq!(stage_kind(&result), expected);
        assert!(
            !reg.transport().log[2].authorized,
            "a record read carries no token"
        );
    }
}

#[test]
fn version_exists_is_duplicate_unless_the_version_is_sealed_with_another_file_set() {
    let (owned, members, id) = world();
    let record = record_path("alpha", "1.0.0");
    let both = ["linux-64", "noarch"];
    let cases = [
        // Held, yanked or burned: the old bytes stand.
        (
            json_reply(200, &record_json("alpha", "1.0.0", "published", &both)),
            "duplicate",
        ),
        (
            json_reply(200, &record_json("alpha", "1.0.0", "yanked", &both)),
            "duplicate",
        ),
        (
            json_reply(200, &record_json("alpha", "1.0.0", "tombstoned", &[])),
            "duplicate",
        ),
        // Sealed with a file set that lacks a requested coordinate.
        (
            json_reply(
                200,
                &record_json("alpha", "1.0.0", "published", &["linux-64"]),
            ),
            "refused",
        ),
        (
            json_reply(200, &record_json("alpha", "1.0.0", "yanked", &["noarch"])),
            "refused",
        ),
        // A record that cannot be read is no evidence either way.
        (error_reply(404, "version_not_found"), "transient"),
        (error_reply(502, "bad_gateway"), "transient"),
    ];
    for (reply, expected) in cases {
        let fake = no_group(&id)
            .on(
                Method::Post,
                GROUPS,
                vec![error_reply(409, "version_exists")],
            )
            .on(Method::Get, &record, vec![reply]);
        let (result, _) = stage_of(fake, &owned, &members);
        assert_eq!(stage_kind(&result), expected);
    }
}

#[test]
fn a_lost_create_answer_is_transient_and_the_next_stage_resumes_the_same_group() {
    let (owned, members, id) = world();
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let open = group_json(&id, "open", &members, &[]);
    let fake = puts(
        Fake::default()
            .on(
                Method::Get,
                &group_path(&id),
                vec![error_reply(404, "group_not_found"), json_reply(200, &open)],
            )
            .on(
                Method::Post,
                GROUPS,
                vec![lost(), created(&id, &members, &[])],
            ),
        &[ok(200)],
    );
    let mut reg = registry(fake, &fresh());
    let first = reg.stage(&caller(), &members, &files);
    assert_eq!(stage_kind(&first), "transient");
    assert_eq!(reg.transport().count(Method::Put, &put_path("noarch")), 0);
    let second = reg.stage(&caller(), &members, &files);
    assert_eq!(second, Ok(StageAnswer::Staged));
    let bodies: Vec<&str> = reg
        .transport()
        .log
        .iter()
        .filter(|l| l.method == Method::Post)
        .map(|l| l.body.as_str())
        .collect();
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0], bodies[1]);
}

#[test]
fn a_reserving_answer_repeats_the_identical_create_up_to_the_configured_bound() {
    let (owned, members, id) = world();
    let mut still = created_json(&id, "open", &members, &[]);
    still["reserving"] = json!({"done": 0, "total": 1});
    let reserving = json_reply(201, &still);
    // Two partial answers, then the whole one.
    let fake = puts(
        no_group(&id).on(
            Method::Post,
            GROUPS,
            vec![
                reserving.clone(),
                reserving.clone(),
                created(&id, &members, &[]),
            ],
        ),
        &[ok(200)],
    );
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(result, Ok(StageAnswer::Staged));
    let transport = reg.transport();
    assert_eq!(transport.count(Method::Post, GROUPS), 3);
    let bodies: Vec<&String> = transport
        .log
        .iter()
        .filter(|l| l.method == Method::Post)
        .map(|l| &l.body)
        .collect();
    assert!(bodies.iter().all(|b| *b == bodies[0]));
    // A registry that never finishes reserving is asked 1 + reserving_repeats times, then Transient.
    let fake = no_group(&id).on(Method::Post, GROUPS, vec![reserving]);
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(stage_kind(&result), "transient");
    assert_eq!(reg.transport().count(Method::Post, GROUPS), 4);
}

#[test]
fn an_expired_upload_url_repeats_the_create_once_and_retries_the_put() {
    let (owned, members, id) = world();
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![created(&id, &members, &[]), created(&id, &members, &[])],
        )
        .on(Method::Put, &put_path("linux-64"), vec![ok(403), ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(200)]);
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(result, Ok(StageAnswer::Staged));
    assert_eq!(
        reg.transport().lines(),
        [
            format!("GET {}", group_path(&id)),
            format!("POST {GROUPS}"),
            format!("PUT {}", put_path("linux-64")),
            format!("POST {GROUPS}"),
            format!("PUT {}", put_path("linux-64")),
            format!("PUT {}", put_path("noarch")),
        ]
    );
    // A second refusal of the fresh URL is not retried again.
    let fake = no_group(&id)
        .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
        .on(Method::Put, &put_path("linux-64"), vec![ok(403)]);
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(stage_kind(&result), "transient");
    assert_eq!(reg.transport().count(Method::Post, GROUPS), 2);
    assert_eq!(reg.transport().count(Method::Put, &put_path("linux-64")), 2);
}

#[test]
fn an_expired_url_whose_file_the_fresh_answer_lists_as_stored_is_not_put_again() {
    let (owned, members, id) = world();
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![
                created(&id, &members, &[]),
                created(&id, &members, &["linux-64", "noarch"]),
            ],
        )
        .on(Method::Put, &put_path("linux-64"), vec![ok(403)]);
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(result, Ok(StageAnswer::Duplicate));
    assert_eq!(reg.transport().count(Method::Put, &put_path("linux-64")), 1);
    assert_eq!(reg.transport().count(Method::Put, &put_path("noarch")), 0);
}

#[test]
fn an_answer_older_than_half_the_url_life_is_repeated_before_the_next_put() {
    let (owned, members, id) = world();
    let clock = fresh();
    let fake = no_group(&id)
        .on(
            Method::Post,
            GROUPS,
            vec![
                created(&id, &members, &[]),
                created(&id, &members, &["linux-64"]),
            ],
        )
        .on(Method::Put, &put_path("linux-64"), vec![ok(200)])
        .on(Method::Put, &put_path("noarch"), vec![ok(200)])
        .bump(
            Method::Put,
            &put_path("linux-64"),
            &clock,
            Duration::from_secs(200),
        );
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let mut cfg = group_config();
    cfg.upload_budget = Duration::from_secs(3000);
    let mut reg = Registry::new(fake, cfg, Box::new(clock));
    assert_eq!(
        reg.stage(&caller(), &members, &files),
        Ok(StageAnswer::Staged)
    );
    assert_eq!(
        reg.transport().lines(),
        [
            format!("GET {}", group_path(&id)),
            format!("POST {GROUPS}"),
            format!("PUT {}", put_path("linux-64")),
            format!("POST {GROUPS}"),
            format!("PUT {}", put_path("noarch")),
        ]
    );
}

#[test]
fn a_stage_that_runs_out_of_budget_stops_between_files_and_one_with_no_budget_sends_nothing() {
    let (owned, members, id) = world();
    let clock = fresh();
    let fake = puts(
        no_group(&id).on(Method::Post, GROUPS, vec![created(&id, &members, &[])]),
        &[ok(200)],
    )
    .bump(
        Method::Put,
        &put_path("linux-64"),
        &clock,
        Duration::from_secs(400),
    );
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let mut reg = Registry::new(fake, group_config(), Box::new(clock));
    let result = reg.stage(&caller(), &members, &files);
    assert_eq!(stage_kind(&result), "transient");
    assert_eq!(reg.transport().count(Method::Put, &put_path("linux-64")), 1);
    assert_eq!(reg.transport().count(Method::Put, &put_path("noarch")), 0);
    // No time at all: the first request is not sent.
    let mut cfg = group_config();
    cfg.upload_budget = Duration::ZERO;
    let mut reg = Registry::new(Fake::default(), cfg, Box::new(fresh()));
    let result = reg.stage(&caller(), &members, &files);
    assert_eq!(stage_kind(&result), "transient");
    assert!(reg.transport().log.is_empty());
}

#[test]
fn a_put_is_retried_on_a_5xx_or_a_lost_answer_up_to_the_configured_count() {
    let (owned, members, id) = world();
    let base = |replies: Vec<Reply>| {
        no_group(&id)
            .on(Method::Post, GROUPS, vec![created(&id, &members, &[])])
            .on(Method::Put, &put_path("linux-64"), replies)
            .on(Method::Put, &put_path("noarch"), vec![ok(200)])
    };
    let (result, reg) = stage_of(base(vec![ok(503), lost(), ok(200)]), &owned, &members);
    assert_eq!(result, Ok(StageAnswer::Staged));
    assert_eq!(reg.transport().count(Method::Put, &put_path("linux-64")), 3);
    // The configured count is 2 retries: three attempts, then Transient naming them.
    let (result, reg) = stage_of(base(vec![ok(502)]), &owned, &members);
    let Err(SeamError::Transient(message)) = result else {
        panic!("a spent retry count is Transient");
    };
    assert!(message.contains("3 tries"), "{message}");
    assert_eq!(reg.transport().count(Method::Put, &put_path("linux-64")), 3);
    // Any other refusal of storage is not retried.
    let (result, reg) = stage_of(base(vec![ok(400)]), &owned, &members);
    assert_eq!(stage_kind(&result), "transient");
    assert_eq!(reg.transport().count(Method::Put, &put_path("linux-64")), 1);
}

#[test]
fn an_upload_url_on_an_origin_that_is_not_configured_is_never_put_to() {
    let (owned, members, id) = world();
    let mut value = created_json(&id, "open", &members, &[]);
    value["members"][0]["files"][0]["put_url"] = json!("https://evil.example.net/x?sig=1");
    let fake = puts(
        no_group(&id).on(Method::Post, GROUPS, vec![json_reply(201, &value)]),
        &[ok(200)],
    );
    let (result, reg) = stage_of(fake, &owned, &members);
    assert_eq!(stage_kind(&result), "transient");
    assert_eq!(reg.transport().count(Method::Put, &put_path("linux-64")), 0);
    assert!(reg.transport().log.iter().all(|l| l.method != Method::Put));
}

#[test]
fn a_create_answer_naming_another_group_aborts_it_and_is_permanent_or_unknown() {
    let (owned, members, id) = world();
    let drifted = json_reply(201, &created_json(OTHER, "open", &members, &[]));
    let route = |delete: Vec<Reply>| {
        let fake = no_group(&id).on(Method::Post, GROUPS, vec![drifted.clone()]);
        if delete.is_empty() {
            fake
        } else {
            fake.on(Method::Delete, &group_path(OTHER), delete)
        }
    };
    let (result, reg) = stage_of(route(vec![ok(200)]), &owned, &members);
    assert_eq!(stage_kind(&result), "permanent");
    let log = &reg.transport().log;
    assert_eq!(log.last().unwrap().method, Method::Delete);
    assert_eq!(log.last().unwrap().path, group_path(OTHER));
    assert!(log.last().unwrap().authorized);
    assert!(log.iter().all(|l| l.method != Method::Put));
    // An abort that fails, and one that cannot be sent, leave the effect unknown.
    for delete in [vec![ok(500)], vec![lost()], vec![]] {
        let (result, _) = stage_of(route(delete), &owned, &members);
        assert_eq!(stage_kind(&result), "unknown");
    }
}

#[test]
fn a_superseded_list_under_refuse_is_refused_for_that_stage_and_every_later_one() {
    let (owned, members, id) = world();
    let mut value = created_json(&id, "open", &members, &[]);
    value["superseded"] = json!([OTHER]);
    let fake = puts(
        no_group(&id).on(Method::Post, GROUPS, vec![json_reply(201, &value)]),
        &[ok(200)],
    );
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let mut reg = registry(fake, &fresh());
    let first = reg.stage(&caller(), &members, &files);
    assert_eq!(first, Err(SeamError::Refused(Reason::Channel)));
    let sent = reg.transport().log.len();
    assert_eq!(sent, 2);
    let second = reg.stage(&caller(), &members, &files);
    assert_eq!(second, Err(SeamError::Refused(Reason::Channel)));
    assert_eq!(reg.transport().log.len(), sent);
}

#[test]
fn a_declared_file_without_bytes_or_with_bytes_of_another_size_sends_nothing() {
    let (owned, members, _) = world();
    let bytes = file_bytes();
    let short = vec![7; 999];
    let all = set_files(&owned, &bytes);
    let cases: Vec<Vec<SetFile<'_>>> = vec![
        Vec::new(),
        vec![all[0]],
        vec![
            all[0],
            SetFile {
                bytes: &short,
                ..all[1]
            },
        ],
    ];
    for files in cases {
        let mut reg = registry(Fake::default(), &fresh());
        let result = reg.stage(&caller(), &members, &files);
        assert_eq!(stage_kind(&result), "permanent");
        assert!(reg.transport().log.is_empty());
    }
}

#[test]
fn the_failures_of_a_create_are_classified_by_the_table() {
    let (owned, members, id) = world();
    let retry = Ok(response(
        429,
        &[("retry-after", "7")],
        &error_body("rate_limited"),
    ));
    let cases: Vec<(Reply, Result<StageAnswer, SeamError>)> = vec![
        (retry, Err(SeamError::RateLimited(Backoff { ticks: 7 }))),
        (
            error_reply(401, "token_expired"),
            Err(SeamError::GrantRefused(GrantFault::Expired)),
        ),
        (
            error_reply(401, "token_unknown"),
            Err(SeamError::GrantRefused(GrantFault::Denied)),
        ),
        (
            error_reply(403, "forbidden"),
            Err(SeamError::GrantRefused(GrantFault::Denied)),
        ),
        (
            error_reply(404, "route_not_found"),
            Err(SeamError::Refused(Reason::Channel)),
        ),
        (
            error_reply(404, "package_not_found"),
            Err(SeamError::Refused(Reason::Channel)),
        ),
        (
            error_reply(413, "payload_too_large"),
            Err(SeamError::Refused(Reason::Channel)),
        ),
    ];
    for (reply, expected) in cases {
        let fake = no_group(&id).on(Method::Post, GROUPS, vec![reply]);
        let (result, _) = stage_of(fake, &owned, &members);
        assert_eq!(result, expected);
    }
    let kinds = [
        (error_reply(400, "invalid_request"), "permanent"),
        (error_reply(503, "unavailable"), "transient"),
        (ok(201), "permanent"),
        (lost(), "transient"),
    ];
    for (reply, expected) in kinds {
        let fake = no_group(&id).on(Method::Post, GROUPS, vec![reply]);
        let (result, _) = stage_of(fake, &owned, &members);
        assert_eq!(stage_kind(&result), expected);
    }
}

#[test]
fn a_group_read_that_fails_fails_the_stage_and_a_token_that_cannot_be_sent_sends_nothing() {
    let (owned, members, id) = world();
    let other = json_reply(200, &group_json(OTHER, "open", &members, &[]));
    let cases = [
        (error_reply(500, "internal_error"), "transient"),
        (error_reply(404, "package_not_found"), "transient"),
        (ok(200), "permanent"),
        (other, "permanent"),
        (lost(), "transient"),
        (error_reply(401, "token_unknown"), "grant_refused"),
    ];
    for (reply, expected) in cases {
        let fake = Fake::default().on(Method::Get, &group_path(&id), vec![reply]);
        let (result, reg) = stage_of(fake, &owned, &members);
        assert_eq!(stage_kind(&result), expected);
        assert_eq!(
            reg.transport().log.len(),
            1,
            "no create follows a failed read"
        );
    }
    let bytes = file_bytes();
    let files = set_files(&owned, &bytes);
    let spaced = Caller {
        actor: ACTOR,
        token: b"has a space",
    };
    let mut reg = registry(Fake::default(), &fresh());
    let result = reg.stage(&spaced, &members, &files);
    assert_eq!(result, Err(SeamError::GrantRefused(GrantFault::Denied)));
    assert!(reg.transport().log.is_empty());
}
