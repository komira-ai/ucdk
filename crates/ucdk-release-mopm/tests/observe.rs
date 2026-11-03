// SPDX-License-Identifier: Apache-2.0
//! `observe` against a registry written by hand: the version record, and for a declared set the
//! group the set names, and that a failed read is never an absent coordinate.

mod common;

use common::*;
use serde_json::{Value, json};
use ucdk_core::ManualClock;
use ucdk_release::codec::GrantFault;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_mopm::{Caller, Config, Key, Member, Method, Registry, Subdir, group_id};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Backoff, SeamError};

const BOTH: [&str; 2] = ["linux-64", "noarch"];

fn held() -> Held {
    Held {
        digest: DIGEST.to_owned(),
        stamp: None,
        exceptional: false,
    }
}

fn record_route(fake: Fake, reply: Reply) -> Fake {
    fake.on(Method::Get, &record_path("alpha", "1.0.0"), vec![reply])
}

fn group_route(fake: Fake, id: &str, reply: Reply) -> Fake {
    fake.on(Method::Get, &group_path(id), vec![reply])
}

fn observe(
    fake: Fake,
    subdir: Subdir,
    set: &[Member],
) -> (Result<Presence, SeamError>, Registry<Fake>) {
    let mut reg = registry(fake, &ManualClock::at(1_000_000));
    let result = reg.observe(Some(&caller()), &key(subdir), set);
    (result, reg)
}

fn kind(result: &Result<Presence, SeamError>) -> &'static str {
    match result {
        Ok(Presence::Absent) => "absent",
        Ok(Presence::Staged(_)) => "staged",
        Ok(Presence::Present(_)) => "present",
        Ok(Presence::Yanked(_)) => "yanked",
        Ok(Presence::Burned) => "burned",
        Err(error) => err_kind(error),
    }
}

fn not_found() -> Reply {
    error_reply(404, "version_not_found")
}

#[test]
fn a_declared_set_reads_the_group_then_the_record_and_the_record_wins() {
    let (_, members, id) = world();
    let open = json_reply(200, &group_json(&id, "open", &members, &BOTH));
    let record = json_reply(200, &record_json("alpha", "1.0.0", "published", &BOTH));
    let fake = record_route(group_route(Fake::default(), &id, open), record);
    let (result, reg) = observe(fake, Subdir::Linux64, &members);
    assert_eq!(result, Ok(Presence::Present(held())));
    assert_eq!(
        reg.transport().lines(),
        [
            format!("GET {}", group_path(&id)),
            format!("GET {}", record_path("alpha", "1.0.0")),
        ]
    );
    assert!(reg.transport().log[0].authorized);
    assert!(!reg.transport().log[1].authorized);
}

#[test]
fn staged_is_an_open_group_listing_exactly_the_declared_files_all_stored() {
    let (_, members, id) = world();
    let read = |status: &str, stored: &[&str]| {
        let group = json_reply(200, &group_json(&id, status, &members, stored));
        let fake = record_route(group_route(Fake::default(), &id, group), not_found());
        observe(fake, Subdir::Noarch, &members).0
    };
    assert_eq!(read("open", &BOTH), Ok(Presence::Staged(held())));
    // A file that is not stored, a group past open and a group that does not exist are absent.
    assert_eq!(read("open", &["noarch"]), Ok(Presence::Absent));
    assert_eq!(read("open", &[]), Ok(Presence::Absent));
    for status in [
        "validating",
        "committed",
        "visible",
        "rejected",
        "failed",
        "expired",
        "aborted",
        "superseded",
    ] {
        assert_eq!(read(status, &BOTH), Ok(Presence::Absent), "{status}");
    }
    let fake = record_route(
        group_route(Fake::default(), &id, error_reply(404, "group_not_found")),
        not_found(),
    );
    assert_eq!(
        observe(fake, Subdir::Noarch, &members).0,
        Ok(Presence::Absent)
    );
}

#[test]
fn a_member_the_group_lists_no_files_for_is_not_reserved_yet_and_absent() {
    let (_, members, id) = world();
    let mut value = group_json(&id, "open", &members, &BOTH);
    value["members"][0]["files"] = json!([]);
    let fake = record_route(
        group_route(Fake::default(), &id, json_reply(200, &value)),
        not_found(),
    );
    assert_eq!(
        observe(fake, Subdir::Linux64, &members).0,
        Ok(Presence::Absent)
    );
    // A group that lists no such member says the same.
    value["members"] = json!([]);
    let fake = record_route(
        group_route(Fake::default(), &id, json_reply(200, &value)),
        not_found(),
    );
    assert_eq!(
        observe(fake, Subdir::Linux64, &members).0,
        Ok(Presence::Absent)
    );
}

#[test]
fn a_group_that_lists_other_files_than_the_declared_ones_is_a_broken_response() {
    let (_, members, id) = world();
    let mut other_digest = members.clone();
    other_digest[0].files[0].sha256 = "f".repeat(64);
    let mut other_size = members.clone();
    other_size[0].files[1].size = 7;
    let mut fewer = members.clone();
    fewer[0].files.pop();
    for listed in [other_digest, other_size, fewer] {
        let read = json_reply(200, &group_json(&id, "open", &listed, &BOTH));
        let fake = record_route(group_route(Fake::default(), &id, read), not_found());
        let (result, _) = observe(fake, Subdir::Linux64, &members);
        assert_eq!(kind(&result), "permanent");
    }
}

#[test]
fn an_empty_set_reads_only_the_record_and_never_reports_staged_state() {
    let mut reg = registry(
        record_route(Fake::default(), not_found()),
        &ManualClock::at(1_000_000),
    );
    assert_eq!(
        reg.observe(None, &key(Subdir::Linux64), &[]),
        Ok(Presence::Absent)
    );
    assert_eq!(
        reg.transport().lines(),
        [format!("GET {}", record_path("alpha", "1.0.0"))]
    );
    // A package that does not exist is as absent as a version that does not.
    let fake = record_route(Fake::default(), error_reply(404, "package_not_found"));
    let mut reg = registry(fake, &ManualClock::at(1_000_000));
    assert_eq!(
        reg.observe(None, &key(Subdir::Linux64), &[]),
        Ok(Presence::Absent)
    );
}

#[test]
fn a_set_that_does_not_hold_the_coordinate_is_refused_and_a_set_without_a_caller_is_denied() {
    let (_, members, _) = world();
    let other = set(&[Owned::new("beta", "1.0.0", "linux-x86_64")]);
    let osx = key(Subdir::OsxArm64);
    for (held_set, k) in [(&other, key(Subdir::Linux64)), (&members, osx)] {
        let mut reg = registry(Fake::default(), &ManualClock::at(1_000_000));
        let result = reg.observe(Some(&caller()), &k, held_set);
        assert_eq!(result, Err(SeamError::Refused(Reason::Channel)));
        assert!(reg.transport().log.is_empty());
    }
    let mut reg = registry(Fake::default(), &ManualClock::at(1_000_000));
    let result = reg.observe(None, &key(Subdir::Linux64), &members);
    assert_eq!(result, Err(SeamError::GrantRefused(GrantFault::Denied)));
    assert!(reg.transport().log.is_empty());
}

#[test]
fn each_state_of_a_record_is_read_as_its_presence() {
    let linux = |state: &str, subdirs: &[&str]| {
        let record = json_reply(200, &record_json("alpha", "1.0.0", state, subdirs));
        observe(record_route(Fake::default(), record), Subdir::Linux64, &[]).0
    };
    assert_eq!(linux("published", &BOTH), Ok(Presence::Present(held())));
    assert_eq!(linux("yanked", &BOTH), Ok(Presence::Yanked(held())));
    // A takedown keeps its digest and is a withdrawal; a burned number has no file and is its own state.
    assert_eq!(linux("tombstoned", &BOTH), Ok(Presence::Yanked(held())));
    assert_eq!(linux("tombstoned", &[]), Ok(Presence::Burned));
    // A version sealed with another file set holds no such coordinate.
    assert_eq!(linux("published", &["noarch"]), Ok(Presence::Absent));
    assert_eq!(linux("yanked", &["noarch"]), Ok(Presence::Absent));
    assert_eq!(linux("tombstoned", &["noarch"]), Ok(Presence::Absent));
}

#[test]
fn a_read_that_fails_fails_the_observe_and_is_never_an_absent_coordinate() {
    let (_, members, id) = world();
    let gateway = || text_reply(404, "<html>not found</html>");
    let failures = [
        (gateway(), "transient"),
        (error_reply(404, "not_in_index"), "transient"),
        (error_reply(500, "internal_error"), "transient"),
        (error_reply(404, "route_not_found"), "refused"),
        (lost(), "transient"),
        (ok(200), "permanent"),
    ];
    for (reply, expected) in failures {
        // The record read, with no set.
        let fake = record_route(Fake::default(), reply.clone());
        let (result, _) = observe(fake, Subdir::Linux64, &[]);
        assert_eq!(kind(&result), expected);
        // The group read, with the set: the record is not read after it.
        let fake = group_route(Fake::default(), &id, reply);
        let (result, reg) = observe(fake, Subdir::Linux64, &members);
        assert_eq!(kind(&result), expected);
        assert_eq!(reg.transport().log.len(), 1);
    }
    // The record read fails after a group read that was fine.
    let open = json_reply(200, &group_json(&id, "open", &members, &BOTH));
    let fake = record_route(
        group_route(Fake::default(), &id, open),
        error_reply(500, "internal_error"),
    );
    let (result, _) = observe(fake, Subdir::Linux64, &members);
    assert_eq!(kind(&result), "transient");
}

#[test]
fn a_record_of_another_version_or_a_malformed_one_is_a_broken_response() {
    let wrong: Value = record_json("alpha", "1.0.1", "published", &BOTH);
    let broken = json!({"name": "alpha", "version": "1.0.0", "state": "published"});
    for body in [wrong, broken] {
        let fake = record_route(Fake::default(), json_reply(200, &body));
        let (result, _) = observe(fake, Subdir::Linux64, &[]);
        assert_eq!(kind(&result), "permanent");
    }
}

const OTHER: &str = "malformed answer: a group lists files other than the declared ones";
const BAD_RECORD: &str = "malformed answer: a version record";
const BAD_GROUP: &str = "malformed answer: a group read";

fn other_digest() -> String {
    "b".repeat(64)
}

fn permanent(text: &str) -> Result<Presence, SeamError> {
    Err(SeamError::Permanent(text.to_owned()))
}

fn staged_digest(digest: &str) -> Presence {
    Presence::Staged(Held {
        digest: digest.to_owned(),
        stamp: None,
        exceptional: false,
    })
}

fn held_digest(digest: &str) -> Held {
    Held {
        digest: digest.to_owned(),
        stamp: None,
        exceptional: false,
    }
}

/// An observe of the default set where the group read answers `group` and the record is missing.
fn staged_read(group: &Value, subdir: Subdir) -> Result<Presence, SeamError> {
    let (_, members, id) = world();
    let fake = record_route(
        group_route(Fake::default(), &id, json_reply(200, group)),
        not_found(),
    );
    observe(fake, subdir, &members).0
}

#[test]
fn a_group_that_lists_one_declared_file_twice_in_place_of_the_other_is_a_broken_response() {
    let (_, members, id) = world();
    for keep in [0, 1] {
        let mut value = group_json(&id, "open", &members, &BOTH);
        let file = value["members"][0]["files"][keep].clone();
        value["members"][0]["files"] = json!([file, file]);
        assert_eq!(staged_read(&value, Subdir::Linux64), permanent(OTHER));
        assert_eq!(staged_read(&value, Subdir::Noarch), permanent(OTHER));
    }
}

#[test]
fn a_group_that_lists_the_member_twice_is_a_broken_response_even_if_both_copies_are_valid() {
    let (_, members, id) = world();
    let mut value = group_json(&id, "open", &members, &BOTH);
    let member = value["members"][0].clone();
    value["members"] = json!([member, member]);
    assert_eq!(staged_read(&value, Subdir::Linux64), permanent(OTHER));
    // The second copy being the broken one does not hide it behind the first.
    let mut value = group_json(&id, "open", &members, &BOTH);
    let mut second = value["members"][0].clone();
    second["files"] = json!([]);
    let first = value["members"][0].clone();
    value["members"] = json!([first, second]);
    assert_eq!(staged_read(&value, Subdir::Linux64), permanent(OTHER));
}

#[test]
fn the_digest_a_record_gives_is_the_one_observed_and_a_staged_file_has_the_declared_one() {
    let (_, members, id) = world();
    let open = || json_reply(200, &group_json(&id, "open", &members, &BOTH));
    for (state, expected) in [
        ("published", Presence::Present(held_digest(&other_digest()))),
        ("yanked", Presence::Yanked(held_digest(&other_digest()))),
        ("tombstoned", Presence::Yanked(held_digest(&other_digest()))),
    ] {
        let mut record = record_json("alpha", "1.0.0", state, &BOTH);
        record["files"][0]["sha256"] = json!(other_digest());
        let fake = record_route(
            group_route(Fake::default(), &id, open()),
            json_reply(200, &record),
        );
        assert_eq!(observe(fake, Subdir::Linux64, &members).0, Ok(expected));
        // The other file of the record keeps its own digest.
        let mut record = record_json("alpha", "1.0.0", state, &BOTH);
        record["files"][0]["sha256"] = json!(other_digest());
        let fake = record_route(
            group_route(Fake::default(), &id, open()),
            json_reply(200, &record),
        );
        let held_noarch = match observe(fake, Subdir::Noarch, &members).0 {
            Ok(Presence::Present(h) | Presence::Yanked(h)) => h,
            other => panic!("{state}: {other:?}"),
        };
        assert_eq!(held_noarch, held());
    }
    let fake = record_route(group_route(Fake::default(), &id, open()), not_found());
    assert_eq!(
        observe(fake, Subdir::Linux64, &members).0,
        Ok(staged_digest(DIGEST))
    );
}

#[test]
fn a_record_wins_over_a_fully_stored_open_group_in_every_state() {
    let (_, members, id) = world();
    let cases: [(&str, &[&str], Presence); 5] = [
        ("published", &BOTH, Presence::Present(held())),
        ("yanked", &BOTH, Presence::Yanked(held())),
        ("tombstoned", &BOTH, Presence::Yanked(held())),
        ("tombstoned", &[], Presence::Burned),
        ("published", &["noarch"], Presence::Absent),
    ];
    for (state, subdirs, expected) in cases {
        let open = json_reply(200, &group_json(&id, "open", &members, &BOTH));
        let record = json_reply(200, &record_json("alpha", "1.0.0", state, subdirs));
        let fake = record_route(group_route(Fake::default(), &id, open), record);
        let (result, reg) = observe(fake, Subdir::Linux64, &members);
        assert_eq!(result, Ok(expected), "{state} {subdirs:?}");
        assert_eq!(reg.transport().log.len(), 2);
    }
    // The same group with no record is staged.
    let open = json_reply(200, &group_json(&id, "open", &members, &BOTH));
    let fake = record_route(group_route(Fake::default(), &id, open), not_found());
    assert_eq!(
        observe(fake, Subdir::Linux64, &members).0,
        Ok(Presence::Staged(held()))
    );
}

#[test]
fn a_group_that_does_not_match_is_not_judged_while_a_record_exists() {
    let (_, members, id) = world();
    let mut value = group_json(&id, "open", &members, &BOTH);
    value["members"][0]["files"][0]["sha256"] = json!(other_digest());
    // Without a record the mismatch is a broken response.
    assert_eq!(staged_read(&value, Subdir::Linux64), permanent(OTHER));
    // With one the record answers and the listing is never compared.
    let record = json_reply(200, &record_json("alpha", "1.0.0", "published", &BOTH));
    let fake = record_route(
        group_route(Fake::default(), &id, json_reply(200, &value)),
        record,
    );
    let (result, reg) = observe(fake, Subdir::Linux64, &members);
    assert_eq!(result, Ok(Presence::Present(held())));
    assert_eq!(reg.transport().log.len(), 2);
}

#[test]
fn a_set_of_another_caller_in_reverse_order_reads_its_own_group_member_by_member() {
    let owned = vec![
        Owned::new("alpha", "1.0.0", "linux-x86_64"),
        Owned::new("beta", "2.0.0", "linux-x86_64"),
        Owned::new("beta", "2.0.0", "any"),
    ];
    let forward = set(&owned);
    let mut reversed = forward.clone();
    reversed.reverse();
    for member in &mut reversed {
        member.files.reverse();
    }
    let other = Caller {
        actor: "ci:gh-repo:777",
        token: b"OTHER-token-9",
    };
    let id = group_id("ci:gh-repo:777", &forward);
    assert_eq!(id, group_id("ci:gh-repo:777", &reversed));
    assert_ne!(id, group_id(ACTOR, &forward));
    // alpha is fully stored; beta has only its linux-64 file stored.
    let value = group_json(&id, "open", &forward, &["linux-64"]);
    let read = |name: &str, version: &str, subdir: Subdir| {
        let fake = Fake::default()
            .on(Method::Get, &group_path(&id), vec![json_reply(200, &value)])
            .on(Method::Get, &record_path(name, version), vec![not_found()]);
        let mut reg = registry(fake, &ManualClock::at(1_000_000));
        let k = Key {
            name: name.to_owned(),
            version: version.to_owned(),
            subdir,
        };
        let result = reg.observe(Some(&other), &k, &reversed);
        (result, reg.transport().log.clone())
    };
    let (alpha, log) = read("alpha", "1.0.0", Subdir::Linux64);
    assert_eq!(alpha, Ok(Presence::Staged(held())));
    assert_eq!(log[0].path, group_path(&id));
    assert_eq!(log[0].bearer.as_deref(), Some("Bearer OTHER-token-9"));
    assert_eq!(log[1].path, record_path("alpha", "1.0.0"));
    assert_eq!(log[1].bearer, None);
    for subdir in [Subdir::Linux64, Subdir::Noarch] {
        let (beta, log) = read("beta", "2.0.0", subdir);
        assert_eq!(beta, Ok(Presence::Absent), "{subdir:?}");
        assert_eq!(log[0].path, group_path(&id));
        assert_eq!(log[0].bearer.as_deref(), Some("Bearer OTHER-token-9"));
    }
}

#[test]
fn a_listing_with_a_changed_file_name_a_changed_subdirectory_or_an_extra_file_is_broken() {
    let (_, members, id) = world();
    let group = || group_json(&id, "open", &members, &BOTH);
    let mut renamed = group();
    renamed["members"][0]["files"][0]["filename"] = json!("mojo-alpha-1.0.0-other.conda");
    let mut moved = group();
    moved["members"][0]["files"][0]["subdir"] = json!("linux-aarch64");
    let mut extra = group();
    let mut file = extra["members"][0]["files"][0].clone();
    file["subdir"] = json!("osx-arm64");
    extra["members"][0]["files"]
        .as_array_mut()
        .unwrap()
        .push(file);
    for value in [renamed, moved, extra] {
        assert_eq!(staged_read(&value, Subdir::Linux64), permanent(OTHER));
    }
}

#[test]
fn a_listing_of_the_declared_files_in_another_order_is_staged() {
    let (_, members, id) = world();
    let mut value = group_json(&id, "open", &members, &BOTH);
    value["members"][0]["files"]
        .as_array_mut()
        .unwrap()
        .reverse();
    assert_eq!(
        staged_read(&value, Subdir::Linux64),
        Ok(Presence::Staged(held()))
    );
    assert_eq!(
        staged_read(&value, Subdir::Noarch),
        Ok(Presence::Staged(held()))
    );
}

fn read_with(
    config: Config,
    group: Reply,
    record: Option<Reply>,
) -> (Result<Presence, SeamError>, Vec<String>) {
    let (_, members, id) = world();
    let mut fake = group_route(Fake::default(), &id, group);
    if let Some(record) = record {
        fake = record_route(fake, record);
    }
    let mut reg = declared_registry(fake, config, &ManualClock::at(1_000_000));
    let result = reg.observe(Some(&caller()), &key(Subdir::Linux64), &members);
    (result, reg.transport().lines())
}

fn limited(seconds: &str) -> Reply {
    Ok(response(
        429,
        &[("retry-after", seconds)],
        &error_body("rate_limited"),
    ))
}

#[test]
fn an_expired_or_refused_token_on_a_read_is_a_grant_refusal_and_no_later_request_is_sent() {
    let (_, members, id) = world();
    let group_line = format!("GET {}", group_path(&id));
    let record_line = format!("GET {}", record_path("alpha", "1.0.0"));
    let replies = [
        (error_reply(401, "token_expired"), GrantFault::Expired),
        (error_reply(401, "unauthorized"), GrantFault::Denied),
        (error_reply(403, "forbidden"), GrantFault::Denied),
        (ok(401), GrantFault::Denied),
    ];
    for (reply, fault) in replies {
        let (result, lines) = read_with(group_config(), reply.clone(), Some(not_found()));
        assert_eq!(result, Err(SeamError::GrantRefused(fault)));
        assert_eq!(lines, std::slice::from_ref(&group_line));
        // The record read is public, and its refusal is mapped the same way.
        let open = json_reply(200, &group_json(&id, "open", &members, &BOTH));
        let (result, lines) = read_with(group_config(), open, Some(reply));
        assert_eq!(result, Err(SeamError::GrantRefused(fault)));
        assert_eq!(lines, [group_line.clone(), record_line.clone()]);
    }
}

#[test]
fn a_rate_limit_on_a_read_waits_up_to_the_ceiling_and_is_refused_above_it() {
    let (_, members, id) = world();
    let mut config = group_config();
    config.retry_after_ceiling = secs(10);
    let group_line = format!("GET {}", group_path(&id));
    let record_line = format!("GET {}", record_path("alpha", "1.0.0"));
    let waits = [
        (
            "7",
            Err(SeamError::RateLimited(Backoff::after_millis(7000))),
        ),
        (
            "10",
            Err(SeamError::RateLimited(Backoff::after_millis(10_000))),
        ),
        ("11", Err(SeamError::Refused(Reason::Channel))),
        ("86400", Err(SeamError::Refused(Reason::Channel))),
    ];
    for (seconds, expected) in waits {
        let (result, lines) = read_with(config.clone(), limited(seconds), Some(not_found()));
        assert_eq!(result, expected, "group read, {seconds}");
        assert_eq!(lines, std::slice::from_ref(&group_line));
        let open = json_reply(200, &group_json(&id, "open", &members, &BOTH));
        let (result, lines) = read_with(config.clone(), open, Some(limited(seconds)));
        assert_eq!(result, expected, "record read, {seconds}");
        assert_eq!(lines, [group_line.clone(), record_line.clone()]);
    }
}

#[test]
fn a_token_that_cannot_be_sent_is_a_denied_grant_and_nothing_is_sent() {
    let (_, members, _) = world();
    let tokens: [&[u8]; 4] = [b"", b"two\nlines", b"has space", "tok\u{e9}n".as_bytes()];
    for token in tokens {
        let bad = Caller {
            actor: ACTOR,
            token,
        };
        let mut reg = registry(
            record_route(Fake::default(), not_found()),
            &ManualClock::at(1_000_000),
        );
        let result = reg.observe(Some(&bad), &key(Subdir::Linux64), &members);
        assert_eq!(result, Err(SeamError::GrantRefused(GrantFault::Denied)));
        assert!(reg.transport().log.is_empty());
    }
}

#[test]
fn a_malformed_record_or_one_of_another_identity_after_a_good_group_read_is_broken() {
    let (_, members, id) = world();
    let group_line = format!("GET {}", group_path(&id));
    let record_line = format!("GET {}", record_path("alpha", "1.0.0"));
    let mut files_twice = record_json("alpha", "1.0.0", "published", &BOTH);
    files_twice["files"][1]["subdir"] = json!("linux-64");
    let bodies = [
        record_json("alpha", "1.0.1", "published", &BOTH),
        record_json("beta", "1.0.0", "published", &BOTH),
        json!({"name": "alpha", "version": "1.0.0", "state": "published"}),
        record_json("alpha", "1.0.0", "melted", &BOTH),
        files_twice,
    ];
    for body in bodies {
        let open = json_reply(200, &group_json(&id, "open", &members, &BOTH));
        let (result, lines) = read_with(group_config(), open, Some(json_reply(200, &body)));
        assert_eq!(result, permanent(BAD_RECORD), "{body}");
        assert_eq!(lines, [group_line.clone(), record_line.clone()]);
    }
    let open = json_reply(200, &group_json(&id, "open", &members, &BOTH));
    let (result, _) = read_with(group_config(), open, Some(text_reply(200, "not json")));
    assert_eq!(result, permanent(BAD_RECORD));
}

#[test]
fn a_group_of_another_id_or_a_malformed_group_is_a_broken_response_and_no_record_is_read() {
    let (_, members, id) = world();
    let group_line = format!("GET {}", group_path(&id));
    let other_id = format!("grp_{}", "0".repeat(32));
    let bodies = [
        group_json(&other_id, "open", &members, &BOTH),
        json!({"group_id": id, "status": "open", "members": "none"}),
        json!({"group_id": id, "status": "frozen", "members": []}),
    ];
    for body in bodies {
        let (result, lines) = read_with(group_config(), json_reply(200, &body), Some(not_found()));
        assert_eq!(result, permanent(BAD_GROUP), "{body}");
        assert_eq!(lines, std::slice::from_ref(&group_line));
    }
}
