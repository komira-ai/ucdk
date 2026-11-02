// SPDX-License-Identifier: Apache-2.0
//! `list` against a registry written by hand: the pages of the search in name order, the record of
//! each latest version, and that a failed read fails the call and never returns a partial list.

mod common;

use common::*;
use serde_json::{Value, json};
use ucdk_core::ManualClock;
use ucdk_release::publish::Held;
use ucdk_release_mopm::{MAX_LIST_PAGES, Method};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{ListScope, Listed, SeamError};

fn held() -> Held {
    Held {
        digest: DIGEST.to_owned(),
        stamp: None,
        exceptional: false,
    }
}

fn kind<T>(result: &Result<T, SeamError>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(error) => err_kind(error),
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
