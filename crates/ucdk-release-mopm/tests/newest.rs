// SPDX-License-Identifier: Apache-2.0
//! `newest` and `list` against a registry written by hand: the package record, the sparse index and
//! the version records, and that a failed read fails the call and never leaves a version out.

mod common;

use common::*;
use serde_json::{Value, json};
use ucdk_core::ManualClock;
use ucdk_release::publish::Held;
use ucdk_release_mopm::{MAX_LIST_PAGES, Method, Registry};
use ucdk_release_proto::v1::{Coordinate, Reason};
use ucdk_release_seams::{ListScope, Listed, SeamError};

const PACKAGE: &str = "/api/v1/packages/alpha";
const INDEX: &str = "/index/al/ph/alpha";
const BOTH: [&str; 2] = ["linux-64", "noarch"];

fn held() -> Held {
    Held {
        digest: DIGEST.to_owned(),
        stamp: None,
        exceptional: false,
    }
}

fn package_json(versions: &[(&str, &str)]) -> Value {
    let versions: Vec<Value> = versions
        .iter()
        .map(|(v, s)| json!({"version": v, "state": s, "published_at": "2026-12-01T00:00:00Z"}))
        .collect();
    json!({"name": "alpha", "state": "active", "versions": versions})
}

fn record(version: &str, state: &str, subdirs: &[&str]) -> Value {
    record_json("alpha", version, state, subdirs)
}

fn newest_of(fake: Fake) -> (Result<Vec<(Coordinate, Held)>, SeamError>, Registry<Fake>) {
    let mut reg = registry(fake, &ManualClock::at(1_000_000));
    let result = reg.newest(CHANNEL, "alpha");
    (result, reg)
}

fn entries(version: &str) -> Vec<(Coordinate, Held)> {
    vec![
        (coordinate("alpha", version, "linux-x86_64"), held()),
        (coordinate("alpha", version, "any"), held()),
    ]
}

fn kind<T>(result: &Result<T, SeamError>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(error) => err_kind(error),
    }
}

#[test]
fn newest_lists_every_file_of_every_version_with_the_digests_of_the_index() {
    let package = json_reply(
        200,
        &package_json(&[("1.0.1", "published"), ("1.0.0", "yanked")]),
    );
    let index = index_body(&[
        record("1.0.0", "yanked", &BOTH),
        record("1.0.1", "published", &BOTH),
    ]);
    let fake = Fake::default().on(Method::Get, PACKAGE, vec![package]).on(
        Method::Get,
        INDEX,
        vec![text_reply(200, &index)],
    );
    let (result, reg) = newest_of(fake);
    let mut expected = entries("1.0.1");
    expected.extend(entries("1.0.0"));
    assert_eq!(result, Ok(expected));
    // Two reads, both public: no version record was needed.
    assert_eq!(
        reg.transport().lines(),
        [format!("GET {PACKAGE}"), format!("GET {INDEX}")]
    );
    assert!(reg.transport().log.iter().all(|l| !l.authorized));
}

#[test]
fn a_version_the_index_does_not_show_yet_is_read_from_its_own_record() {
    let package = json_reply(
        200,
        &package_json(&[("1.0.1", "published"), ("1.0.0", "published")]),
    );
    let index = index_body(&[record("1.0.0", "published", &BOTH)]);
    let fake = Fake::default()
        .on(Method::Get, PACKAGE, vec![package])
        .on(Method::Get, INDEX, vec![text_reply(200, &index)])
        .on(
            Method::Get,
            &record_path("alpha", "1.0.1"),
            vec![json_reply(200, &record("1.0.1", "published", &BOTH))],
        );
    let (result, reg) = newest_of(fake);
    let mut expected = entries("1.0.1");
    expected.extend(entries("1.0.0"));
    assert_eq!(result, Ok(expected));
    assert_eq!(reg.transport().log.len(), 3);
}

#[test]
fn an_index_that_has_not_the_package_yet_falls_back_to_the_records_and_a_failed_index_fails() {
    let package = json_reply(200, &package_json(&[("1.0.0", "published")]));
    let rec = json_reply(200, &record("1.0.0", "published", &BOTH));
    let base = |index: Reply| {
        Fake::default()
            .on(Method::Get, PACKAGE, vec![package.clone()])
            .on(Method::Get, INDEX, vec![index])
            .on(
                Method::Get,
                &record_path("alpha", "1.0.0"),
                vec![rec.clone()],
            )
    };
    let (result, _) = newest_of(base(error_reply(404, "not_in_index")));
    assert_eq!(result, Ok(entries("1.0.0")));
    for (index, expected) in [
        (error_reply(502, "index_unavailable"), "transient"),
        (error_reply(500, "internal_error"), "transient"),
        (lost(), "transient"),
        (ok(200), "permanent"),
        (text_reply(404, "<html></html>"), "transient"),
    ] {
        let (result, _) = newest_of(base(index));
        assert_eq!(kind(&result), expected);
    }
}

#[test]
fn a_package_nobody_published_or_with_no_version_is_an_empty_list() {
    let fake = Fake::default().on(
        Method::Get,
        PACKAGE,
        vec![error_reply(404, "package_not_found")],
    );
    let (result, reg) = newest_of(fake);
    assert_eq!(result, Ok(Vec::new()));
    assert_eq!(reg.transport().log.len(), 1);
    let fake = Fake::default()
        .on(
            Method::Get,
            PACKAGE,
            vec![json_reply(200, &package_json(&[]))],
        )
        .on(Method::Get, INDEX, vec![error_reply(404, "not_in_index")]);
    let (result, _) = newest_of(fake);
    assert_eq!(result, Ok(Vec::new()));
}

#[test]
fn a_burned_version_is_not_listed_and_a_takedown_with_a_digest_is() {
    let package = json_reply(
        200,
        &package_json(&[("1.0.2", "tombstoned"), ("1.0.1", "tombstoned")]),
    );
    let fake = Fake::default()
        .on(Method::Get, PACKAGE, vec![package])
        .on(Method::Get, INDEX, vec![error_reply(404, "not_in_index")])
        .on(
            Method::Get,
            &record_path("alpha", "1.0.2"),
            vec![json_reply(200, &record("1.0.2", "tombstoned", &[]))],
        )
        .on(
            Method::Get,
            &record_path("alpha", "1.0.1"),
            vec![json_reply(200, &record("1.0.1", "tombstoned", &BOTH))],
        );
    let (result, _) = newest_of(fake);
    assert_eq!(result, Ok(entries("1.0.1")));
}

#[test]
fn a_failed_read_of_the_third_of_five_versions_fails_the_call_and_returns_no_partial_list() {
    let versions = [
        ("1.0.5", "published"),
        ("1.0.4", "published"),
        ("1.0.3", "published"),
        ("1.0.2", "published"),
        ("1.0.1", "published"),
    ];
    let mut fake = Fake::default()
        .on(
            Method::Get,
            PACKAGE,
            vec![json_reply(200, &package_json(&versions))],
        )
        .on(Method::Get, INDEX, vec![error_reply(404, "not_in_index")]);
    for (version, _) in versions {
        let reply = if version == "1.0.3" {
            error_reply(500, "internal_error")
        } else {
            json_reply(200, &record(version, "published", &BOTH))
        };
        fake = fake.on(Method::Get, &record_path("alpha", version), vec![reply]);
    }
    let (result, reg) = newest_of(fake);
    assert_eq!(kind(&result), "transient");
    // The reads stopped at the failure.
    assert_eq!(
        reg.transport()
            .count(Method::Get, &record_path("alpha", "1.0.2")),
        0
    );
    // A version the package lists and the registry has no record of is no evidence it is absent.
    let fake = Fake::default()
        .on(
            Method::Get,
            PACKAGE,
            vec![json_reply(200, &package_json(&[("1.0.0", "published")]))],
        )
        .on(Method::Get, INDEX, vec![error_reply(404, "not_in_index")])
        .on(
            Method::Get,
            &record_path("alpha", "1.0.0"),
            vec![error_reply(404, "version_not_found")],
        );
    let (result, _) = newest_of(fake);
    assert_eq!(kind(&result), "transient");
}

#[test]
fn more_versions_than_the_cap_is_an_error_and_never_a_cut_off_list() {
    let package = json_reply(
        200,
        &package_json(&[
            ("1.0.2", "published"),
            ("1.0.1", "published"),
            ("1.0.0", "published"),
        ]),
    );
    let fake = Fake::default().on(Method::Get, PACKAGE, vec![package]);
    let mut cfg = group_config();
    cfg.newest_versions = 2;
    let mut reg = Registry::new(fake, cfg, Box::new(ManualClock::at(1_000_000)));
    let result = reg.newest(CHANNEL, "alpha");
    assert_eq!(kind(&result), "permanent");
    assert_eq!(reg.transport().log.len(), 1);
}

#[test]
fn newest_refuses_what_mopm_cannot_hold_and_a_broken_package_record_is_permanent() {
    for (channel, name) in [("other", "alpha"), (CHANNEL, "Alpha"), (CHANNEL, "a/b")] {
        let mut reg = registry(Fake::default(), &ManualClock::at(1_000_000));
        let result = reg.newest(channel, name);
        assert_eq!(result, Err(SeamError::Refused(Reason::Channel)));
        assert!(reg.transport().log.is_empty());
    }
    for body in [
        ok(200),
        json_reply(
            200,
            &json!({"versions": [{"version": "1.0.0", "state": "pending"}]}),
        ),
        json_reply(
            200,
            &json!({"versions": [{"version": 1, "state": "published"}]}),
        ),
        json_reply(200, &json!({"versions": "none"})),
    ] {
        let (result, _) = newest_of(Fake::default().on(Method::Get, PACKAGE, vec![body]));
        assert_eq!(kind(&result), "permanent");
    }
    for (reply, expected) in [
        (error_reply(500, "internal_error"), "transient"),
        (lost(), "transient"),
        (error_reply(404, "route_not_found"), "refused"),
    ] {
        let (result, _) = newest_of(Fake::default().on(Method::Get, PACKAGE, vec![reply]));
        assert_eq!(kind(&result), expected);
    }
}

#[test]
fn a_sparse_index_that_is_truncated_of_another_record_version_or_package_or_repeats_is_broken() {
    let package = json_reply(200, &package_json(&[("1.0.0", "published")]));
    let good = record("1.0.0", "published", &BOTH);
    let complete = index_body(std::slice::from_ref(&good));
    let truncated = complete.trim_end().to_owned();
    let mut newer = good.clone();
    newer["v"] = json!(2);
    let versioned = format!("{newer}\n");
    let other = {
        let mut other = good.clone();
        other["name"] = json!("beta");
        index_body(&[other])
    };
    let twice = index_body(&[good.clone(), good.clone()]);
    let blank = format!("{complete}\n");
    for body in [truncated, versioned, other, twice, blank] {
        let fake = Fake::default()
            .on(Method::Get, PACKAGE, vec![package.clone()])
            .on(Method::Get, INDEX, vec![text_reply(200, &body)]);
        let (result, _) = newest_of(fake);
        assert_eq!(kind(&result), "permanent", "{body:?}");
    }
}

#[test]
fn the_index_path_of_a_short_name_is_padded_with_underscores() {
    for (name, path) in [("tls", "/index/tl/s_/tls"), ("ab", "/index/ab/__/ab")] {
        let fake = Fake::default()
            .on(
                Method::Get,
                &format!("/api/v1/packages/{name}"),
                vec![json_reply(200, &json!({"versions": []}))],
            )
            .on(Method::Get, path, vec![error_reply(404, "not_in_index")]);
        let mut reg = registry(fake, &ManualClock::at(1_000_000));
        assert_eq!(reg.newest(CHANNEL, name), Ok(Vec::new()));
        assert_eq!(reg.transport().count(Method::Get, path), 1);
    }
}

fn scope(prefix: &str) -> ListScope {
    ListScope {
        channel: CHANNEL.to_owned(),
        prefix: prefix.to_owned(),
    }
}

fn page(packages: &[(&str, Option<&str>)], next: Option<&str>) -> Reply {
    let packages: Vec<Value> = packages
        .iter()
        .map(|(name, latest)| json!({"name": name, "latest": latest, "description": null}))
        .collect();
    json_reply(200, &json!({"packages": packages, "next_cursor": next}))
}

fn listed(name: &str, version: &str) -> Vec<Listed> {
    vec![Listed {
        coordinate: coordinate(name, version, "linux-x86_64"),
        held: held(),
    }]
}

#[test]
fn a_listing_reads_the_pages_in_order_and_the_record_of_each_latest_version() {
    let fake = Fake::default()
        .on(
            Method::Get,
            "/api/v1/packages",
            vec![
                page(
                    &[("alpha", Some("1.0.0")), ("albatross", None)],
                    Some("c 1"),
                ),
                page(&[("alpine", Some("0.2.0")), ("beta", Some("1.0.0"))], None),
            ],
        )
        .on(
            Method::Get,
            &record_path("alpha", "1.0.0"),
            vec![json_reply(
                200,
                &record_json("alpha", "1.0.0", "published", &["linux-64"]),
            )],
        )
        .on(
            Method::Get,
            &record_path("alpine", "0.2.0"),
            vec![json_reply(
                200,
                &record_json("alpine", "0.2.0", "published", &["linux-64"]),
            )],
        );
    let mut reg = registry(fake, &ManualClock::at(1_000_000));
    let result = reg.list(&scope("al"));
    let mut expected = listed("alpha", "1.0.0");
    expected.extend(listed("alpine", "0.2.0"));
    assert_eq!(result, Ok(expected));
    let log = &reg.transport().log;
    assert_eq!(log.len(), 4);
    assert_eq!(
        log[0].url,
        "https://registry.example.com/api/v1/packages?sort=name&limit=100&q=al"
    );
    assert_eq!(
        log[1].url,
        "https://registry.example.com/api/v1/packages?sort=name&limit=100&q=al&cursor=c%201"
    );
    assert!(log.iter().all(|l| !l.authorized));
    // `beta` does not start with the prefix and a package with no latest has nothing to list.
    assert_eq!(
        reg.transport()
            .count(Method::Get, &record_path("beta", "1.0.0")),
        0
    );
}

#[test]
fn a_listing_of_nothing_is_empty_and_a_record_that_is_not_there_yet_is_left_out() {
    let fake = Fake::default().on(Method::Get, "/api/v1/packages", vec![page(&[], None)]);
    let mut reg = registry(fake, &ManualClock::at(1_000_000));
    assert_eq!(reg.list(&scope("")), Ok(Vec::new()));
    let fake = Fake::default()
        .on(
            Method::Get,
            "/api/v1/packages",
            vec![page(&[("alpha", Some("1.0.0"))], None)],
        )
        .on(
            Method::Get,
            &record_path("alpha", "1.0.0"),
            vec![error_reply(404, "version_not_found")],
        );
    let mut reg = registry(fake, &ManualClock::at(1_000_000));
    assert_eq!(reg.list(&scope("")), Ok(Vec::new()));
    assert!(reg.transport().log[0].url.ends_with("&q="));
}

#[test]
fn a_listing_that_fails_midway_fails_whole() {
    let first = page(&[("alpha", Some("1.0.0"))], Some("next"));
    let cases = [
        (vec![error_reply(500, "internal_error")], "transient"),
        (vec![lost()], "transient"),
        (vec![error_reply(404, "route_not_found")], "refused"),
        (vec![ok(200)], "permanent"),
        (
            vec![json_reply(200, &json!({"packages": [], "next_cursor": 1}))],
            "permanent",
        ),
        (
            vec![json_reply(
                200,
                &json!({"packages": [{"name": "alpha", "latest": 1}], "next_cursor": null}),
            )],
            "permanent",
        ),
        (
            vec![first.clone(), error_reply(500, "internal_error")],
            "transient",
        ),
    ];
    for (replies, expected) in cases {
        let fake = Fake::default()
            .on(Method::Get, "/api/v1/packages", replies)
            .on(
                Method::Get,
                &record_path("alpha", "1.0.0"),
                vec![json_reply(
                    200,
                    &record_json("alpha", "1.0.0", "published", &["linux-64"]),
                )],
            );
        let mut reg = registry(fake, &ManualClock::at(1_000_000));
        let result = reg.list(&scope(""));
        assert_eq!(kind(&result), expected);
    }
    // A failed read of a record fails the listing too.
    let fake = Fake::default()
        .on(
            Method::Get,
            "/api/v1/packages",
            vec![page(&[("alpha", Some("1.0.0"))], None)],
        )
        .on(
            Method::Get,
            &record_path("alpha", "1.0.0"),
            vec![error_reply(500, "internal_error")],
        );
    let mut reg = registry(fake, &ManualClock::at(1_000_000));
    assert_eq!(kind(&reg.list(&scope(""))), "transient");
}

#[test]
fn a_listing_of_a_channel_or_prefix_mopm_cannot_hold_sends_nothing_and_too_many_pages_is_an_error()
{
    let long = "a".repeat(65);
    for (channel, prefix) in [
        ("other", "al"),
        (CHANNEL, "Al"),
        (CHANNEL, "a-b"),
        (CHANNEL, long.as_str()),
    ] {
        let mut reg = registry(Fake::default(), &ManualClock::at(1_000_000));
        let result = reg.list(&ListScope {
            channel: channel.to_owned(),
            prefix: prefix.to_owned(),
        });
        assert_eq!(result, Err(SeamError::Refused(Reason::Channel)));
        assert!(reg.transport().log.is_empty());
    }
    let endless = Fake::default().on(
        Method::Get,
        "/api/v1/packages",
        vec![page(&[], Some("again"))],
    );
    let mut reg = registry(endless, &ManualClock::at(1_000_000));
    let result = reg.list(&scope(""));
    assert_eq!(kind(&result), "permanent");
    assert_eq!(reg.transport().log.len(), MAX_LIST_PAGES);
}
