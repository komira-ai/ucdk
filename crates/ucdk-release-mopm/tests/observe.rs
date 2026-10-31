// SPDX-License-Identifier: Apache-2.0
//! `observe` against a registry written by hand: the version record, and for a declared set the
//! group the set names, and that a failed read is never an absent coordinate.

mod common;

use common::*;
use serde_json::{Value, json};
use ucdk_core::ManualClock;
use ucdk_release::codec::GrantFault;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_mopm::{Member, Method, Registry, Subdir};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::SeamError;

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
