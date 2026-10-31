// SPDX-License-Identifier: Apache-2.0
//! The group read and the version read: what each answer of mopm makes of them, and that a failed
//! read is never taken for an absent one.

mod common;

use common::*;
use serde_json::json;
use ucdk_core::ManualClock;
use ucdk_release::codec::GrantFault;
use ucdk_release_mopm::{GroupStatus, Method, Registry, VersionState};
use ucdk_release_seams::SeamError;

fn reg(fake: Fake) -> Registry<Fake> {
    registry(fake, &ManualClock::at(1_000_000))
}

#[test]
fn a_group_read_gives_the_group_and_only_a_404_group_not_found_is_no_group() {
    let (_, members, id) = world();
    let read = group_json(&id, "open", &members, &["linux-64"]);
    let fake = Fake::default().on(Method::Get, &group_path(&id), vec![json_reply(200, &read)]);
    let mut r = reg(fake);
    let group = r.read_group(&caller(), &id).unwrap().unwrap();
    assert_eq!(group.id, id);
    assert_eq!(group.status, GroupStatus::Open);
    assert!(r.transport().log[0].authorized);
    let fake = Fake::default().on(
        Method::Get,
        &group_path(&id),
        vec![error_reply(404, "group_not_found")],
    );
    assert_eq!(reg(fake).read_group(&caller(), &id), Ok(None));
}

#[test]
fn a_failed_group_read_is_an_error_and_never_no_group() {
    let (_, members, id) = world();
    let wrong = json_reply(200, &group_json(&id, "open", &members, &[]));
    let gateway = Ok(response(404, &[], "<html>not found</html>"));
    let cases = [
        (gateway, "transient"),
        (error_reply(404, "route_not_found"), "refused"),
        (error_reply(404, "package_not_found"), "transient"),
        (error_reply(500, "internal_error"), "transient"),
        (error_reply(401, "token_unknown"), "grant_refused"),
        (lost(), "transient"),
        (ok(200), "permanent"),
        (
            json_reply(200, &json!({"group_id": id, "status": "open"})),
            "permanent",
        ),
    ];
    for (reply, expected) in cases {
        let fake = Fake::default().on(Method::Get, &group_path(&id), vec![reply]);
        let result = reg(fake).read_group(&caller(), &id);
        let kind = match &result {
            Ok(_) => "ok",
            Err(error) => err_kind(error),
        };
        assert_eq!(kind, expected);
    }
    // A group of another id than the one asked for is a broken response.
    let fake = Fake::default().on(Method::Get, &group_path("grp_other"), vec![wrong]);
    let result = reg(fake).read_group(&caller(), "grp_other");
    assert!(matches!(result, Err(SeamError::Permanent(_))), "{result:?}");
}

#[test]
fn a_version_read_is_public_and_gives_the_record_the_registry_holds() {
    let record = record_json("alpha", "1.0.0", "published", &["linux-64", "noarch"]);
    let path = record_path("alpha", "1.0.0");
    let fake = Fake::default().on(Method::Get, &path, vec![json_reply(200, &record)]);
    let mut r = reg(fake);
    let found = r.read_record("alpha", "1.0.0").unwrap().unwrap();
    assert_eq!(found.state, VersionState::Published);
    assert_eq!(found.files.len(), 2);
    assert_eq!(r.transport().lines(), [format!("GET {path}")]);
    assert!(!r.transport().log[0].authorized);
}

#[test]
fn a_version_read_is_absent_only_for_a_parsed_package_or_version_not_found() {
    let path = record_path("alpha", "1.0.0");
    for code in ["package_not_found", "version_not_found"] {
        let fake = Fake::default().on(Method::Get, &path, vec![error_reply(404, code)]);
        assert_eq!(reg(fake).read_record("alpha", "1.0.0"), Ok(None), "{code}");
    }
    let cases = [
        (Ok(response(404, &[], "<html></html>")), "transient"),
        (error_reply(404, "route_not_found"), "refused"),
        (error_reply(404, "not_in_index"), "transient"),
        (error_reply(500, "internal_error"), "transient"),
        (lost(), "transient"),
        (ok(200), "permanent"),
    ];
    for (reply, expected) in cases {
        let fake = Fake::default().on(Method::Get, &path, vec![reply]);
        let result = reg(fake).read_record("alpha", "1.0.0");
        let kind = match &result {
            Ok(_) => "ok",
            Err(error) => err_kind(error),
        };
        assert_eq!(kind, expected);
    }
    assert_eq!(
        reg(Fake::default().on(Method::Get, &path, vec![error_reply(403, "forbidden")]))
            .read_record("alpha", "1.0.0"),
        Err(SeamError::GrantRefused(GrantFault::Denied))
    );
}

#[test]
fn a_record_for_another_package_or_version_or_with_two_files_for_a_subdirectory_is_broken() {
    let path = record_path("alpha", "1.0.0");
    let other_name = record_json("beta", "1.0.0", "published", &["linux-64"]);
    let other_version = record_json("alpha", "1.0.1", "published", &["linux-64"]);
    let twice = record_json("alpha", "1.0.0", "published", &["linux-64", "linux-64"]);
    for body in [other_name, other_version, twice] {
        let fake = Fake::default().on(Method::Get, &path, vec![json_reply(200, &body)]);
        let result = reg(fake).read_record("alpha", "1.0.0");
        assert!(matches!(result, Err(SeamError::Permanent(_))), "{result:?}");
    }
}
