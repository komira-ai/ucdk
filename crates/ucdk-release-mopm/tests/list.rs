// SPDX-License-Identifier: Apache-2.0
//! `list` against a registry written by hand: the pages of the search in name order, the record of
//! each latest version, and that a failed read fails the call and never returns a partial list.

mod common;

use common::*;
use serde_json::{Value, json};
use ucdk_core::ManualClock;
use ucdk_release::codec::GrantFault;
use ucdk_release::publish::Held;
use ucdk_release_mopm::{MAX_LIST_PAGES, Method};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Backoff, ListScope, Listed, SeamError};

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

const SEARCH: &str = "/api/v1/packages";

fn fresh() -> ManualClock {
    ManualClock::at(1_000_000)
}

fn digest(c: char) -> String {
    c.to_string().repeat(64)
}

fn held_of(sha: &str) -> Held {
    Held {
        digest: sha.to_owned(),
        stamp: None,
        exceptional: false,
    }
}

fn entry(name: &str, version: &str, platform: &str, sha: &str) -> Listed {
    Listed {
        coordinate: coordinate(name, version, platform),
        held: held_of(sha),
    }
}

/// A published record of `name` `version` whose file in each subdir carries the given digest.
fn record_with(name: &str, version: &str, files: &[(&str, &str)]) -> Value {
    let subdirs: Vec<&str> = files.iter().map(|(subdir, _)| *subdir).collect();
    let mut record = record_json(name, version, "published", &subdirs);
    for (i, (_, sha)) in files.iter().enumerate() {
        record["files"][i]["sha256"] = json!(sha);
    }
    record
}

fn record_on(fake: Fake, name: &str, version: &str, reply: Reply) -> Fake {
    fake.on(Method::Get, &record_path(name, version), vec![reply])
}

fn searches(reg: &ucdk_release_mopm::Registry<Fake>) -> usize {
    reg.transport().count(Method::Get, SEARCH)
}

fn record_reads(reg: &ucdk_release_mopm::Registry<Fake>) -> usize {
    reg.transport()
        .log
        .iter()
        .filter(|l| l.path != SEARCH)
        .count()
}

#[test]
fn a_listing_of_several_packages_and_platforms_holds_each_file_with_its_own_digest() {
    let fake = Fake::default().on(
        Method::Get,
        SEARCH,
        vec![
            page(
                &[("alpha", Some("1.0.0")), ("bravo", Some("2.1.0"))],
                Some("n"),
            ),
            page(&[("charlie", Some("0.0.1"))], None),
        ],
    );
    let fake = record_on(
        fake,
        "alpha",
        "1.0.0",
        json_reply(
            200,
            &record_with(
                "alpha",
                "1.0.0",
                &[("linux-64", &digest('a')), ("noarch", &digest('b'))],
            ),
        ),
    );
    let fake = record_on(
        fake,
        "bravo",
        "2.1.0",
        json_reply(
            200,
            &record_with(
                "bravo",
                "2.1.0",
                &[
                    ("linux-aarch64", &digest('c')),
                    ("osx-arm64", &digest('d')),
                    ("linux-64", &digest('e')),
                ],
            ),
        ),
    );
    let fake = record_on(
        fake,
        "charlie",
        "0.0.1",
        json_reply(
            200,
            &record_with("charlie", "0.0.1", &[("noarch", &digest('f'))]),
        ),
    );
    let mut reg = registry(fake, &fresh());
    assert_eq!(
        reg.list(&scope("")),
        Ok(vec![
            entry("alpha", "1.0.0", "linux-x86_64", &digest('a')),
            entry("alpha", "1.0.0", "any", &digest('b')),
            entry("bravo", "2.1.0", "linux-aarch64", &digest('c')),
            entry("bravo", "2.1.0", "darwin-aarch64", &digest('d')),
            entry("bravo", "2.1.0", "linux-x86_64", &digest('e')),
            entry("charlie", "0.0.1", "any", &digest('f')),
        ])
    );
    assert_eq!(
        reg.transport().lines(),
        [
            "GET /api/v1/packages",
            "GET /api/v1/packages",
            "GET /api/v1/packages/alpha/1.0.0",
            "GET /api/v1/packages/bravo/2.1.0",
            "GET /api/v1/packages/charlie/0.0.1",
        ]
    );
}

#[test]
fn a_cursor_that_is_null_on_the_last_page_the_cap_allows_ends_the_listing() {
    let mut replies = vec![page(&[], Some("again")); MAX_LIST_PAGES - 1];
    replies.push(page(&[("alpha", Some("1.0.0"))], None));
    let fake = Fake::default().on(Method::Get, SEARCH, replies);
    let fake = record_on(
        fake,
        "alpha",
        "1.0.0",
        json_reply(
            200,
            &record_json("alpha", "1.0.0", "published", &["linux-64"]),
        ),
    );
    let mut reg = registry(fake, &fresh());
    assert_eq!(reg.list(&scope("")), Ok(listed("alpha", "1.0.0")));
    assert_eq!(searches(&reg), MAX_LIST_PAGES);
    assert_eq!(record_reads(&reg), 1);
}

#[test]
fn an_empty_cursor_is_a_broken_page_and_nothing_more_is_sent() {
    let fake = Fake::default().on(
        Method::Get,
        SEARCH,
        vec![page(&[("alpha", Some("1.0.0"))], Some(""))],
    );
    let mut reg = registry(fake, &fresh());
    assert_eq!(
        reg.list(&scope("")),
        Err(SeamError::Permanent(
            "malformed answer: a page of packages".to_owned()
        ))
    );
    assert_eq!(reg.transport().lines(), ["GET /api/v1/packages"]);
}

#[test]
fn an_empty_page_with_a_cursor_goes_on_to_the_next_page() {
    let fake = Fake::default().on(
        Method::Get,
        SEARCH,
        vec![
            page(&[], Some("second")),
            page(&[("alpha", Some("1.0.0"))], None),
        ],
    );
    let fake = record_on(
        fake,
        "alpha",
        "1.0.0",
        json_reply(
            200,
            &record_json("alpha", "1.0.0", "published", &["linux-64"]),
        ),
    );
    let mut reg = registry(fake, &fresh());
    assert_eq!(reg.list(&scope("")), Ok(listed("alpha", "1.0.0")));
    let log = &reg.transport().log;
    assert_eq!(log.len(), 3);
    assert!(log[1].url.ends_with("&cursor=second"));
}

#[test]
fn a_package_named_twice_in_a_page_or_across_pages_is_a_broken_search() {
    let twice = Err(SeamError::Permanent(
        "malformed answer: the search names a package twice".to_owned(),
    ));
    let same = Fake::default().on(
        Method::Get,
        SEARCH,
        vec![page(
            &[("alpha", Some("1.0.0")), ("alpha", Some("1.0.0"))],
            None,
        )],
    );
    let mut reg = registry(same, &fresh());
    assert_eq!(reg.list(&scope("")), twice);
    assert_eq!(record_reads(&reg), 0);
    let across = Fake::default().on(
        Method::Get,
        SEARCH,
        vec![
            page(&[("alpha", Some("1.0.0"))], Some("n")),
            page(&[("alpha", None)], Some("m")),
            page(&[], None),
        ],
    );
    let mut reg = registry(across, &fresh());
    assert_eq!(reg.list(&scope("")), twice);
    assert_eq!(searches(&reg), 2);
    assert_eq!(record_reads(&reg), 0);
}

#[test]
fn a_record_that_is_missing_is_left_out_and_the_ones_around_it_are_kept() {
    let names = [
        ("alpha", Some("1.0.0")),
        ("bravo", Some("1.0.0")),
        ("gamma", Some("1.0.0")),
    ];
    let fake = Fake::default().on(Method::Get, SEARCH, vec![page(&names, None)]);
    let fake = record_on(
        fake,
        "alpha",
        "1.0.0",
        json_reply(
            200,
            &record_with("alpha", "1.0.0", &[("linux-64", &digest('a'))]),
        ),
    );
    let fake = record_on(
        fake,
        "bravo",
        "1.0.0",
        error_reply(404, "version_not_found"),
    );
    let fake = record_on(
        fake,
        "gamma",
        "1.0.0",
        json_reply(
            200,
            &record_with("gamma", "1.0.0", &[("noarch", &digest('9'))]),
        ),
    );
    let mut reg = registry(fake, &fresh());
    assert_eq!(
        reg.list(&scope("")),
        Ok(vec![
            entry("alpha", "1.0.0", "linux-x86_64", &digest('a')),
            entry("gamma", "1.0.0", "any", &digest('9')),
        ])
    );
    assert_eq!(record_reads(&reg), 3);
}

#[test]
fn a_failed_record_read_stops_the_listing_before_the_next_record() {
    let names = [
        ("alpha", Some("1.0.0")),
        ("bravo", Some("1.0.0")),
        ("gamma", Some("1.0.0")),
    ];
    let fake = Fake::default().on(Method::Get, SEARCH, vec![page(&names, None)]);
    let fake = record_on(
        fake,
        "alpha",
        "1.0.0",
        json_reply(
            200,
            &record_json("alpha", "1.0.0", "published", &["linux-64"]),
        ),
    );
    let fake = record_on(fake, "bravo", "1.0.0", error_reply(503, "unavailable"));
    let fake = record_on(
        fake,
        "gamma",
        "1.0.0",
        json_reply(
            200,
            &record_json("gamma", "1.0.0", "published", &["linux-64"]),
        ),
    );
    let mut reg = registry(fake, &fresh());
    assert_eq!(
        reg.list(&scope("")),
        Err(SeamError::Transient(
            "mopm answered 503 unavailable".to_owned()
        ))
    );
    assert_eq!(
        reg.transport()
            .count(Method::Get, &record_path("alpha", "1.0.0")),
        1
    );
    assert_eq!(
        reg.transport()
            .count(Method::Get, &record_path("bravo", "1.0.0")),
        1
    );
    assert_eq!(
        reg.transport()
            .count(Method::Get, &record_path("gamma", "1.0.0")),
        0
    );
}

#[test]
fn an_older_published_version_named_latest_is_listed_with_its_own_record() {
    // The package has a newer yanked 2.0.0; the search names 1.0.0 and only that record is read.
    let fake = Fake::default().on(
        Method::Get,
        SEARCH,
        vec![page(&[("alpha", Some("1.0.0"))], None)],
    );
    let fake = record_on(
        fake,
        "alpha",
        "1.0.0",
        json_reply(
            200,
            &record_with("alpha", "1.0.0", &[("linux-64", &digest('1'))]),
        ),
    );
    let fake = record_on(
        fake,
        "alpha",
        "2.0.0",
        json_reply(200, &record_json("alpha", "2.0.0", "yanked", &["linux-64"])),
    );
    let mut reg = registry(fake, &fresh());
    assert_eq!(
        reg.list(&scope("")),
        Ok(vec![entry("alpha", "1.0.0", "linux-x86_64", &digest('1'))])
    );
    assert_eq!(
        reg.transport()
            .count(Method::Get, &record_path("alpha", "2.0.0")),
        0
    );
}

#[test]
fn a_latest_withdrawn_between_the_search_and_the_record_read_is_left_out() {
    let names = [
        ("alpha", Some("1.0.0")),
        ("bravo", Some("1.0.0")),
        ("gamma", Some("1.0.0")),
    ];
    let fake = Fake::default().on(Method::Get, SEARCH, vec![page(&names, None)]);
    let fake = record_on(
        fake,
        "alpha",
        "1.0.0",
        json_reply(200, &record_json("alpha", "1.0.0", "yanked", &["linux-64"])),
    );
    let fake = record_on(
        fake,
        "bravo",
        "1.0.0",
        json_reply(
            200,
            &record_json("bravo", "1.0.0", "tombstoned", &["linux-64"]),
        ),
    );
    let fake = record_on(
        fake,
        "gamma",
        "1.0.0",
        json_reply(
            200,
            &record_with("gamma", "1.0.0", &[("noarch", &digest('9'))]),
        ),
    );
    let mut reg = registry(fake, &fresh());
    assert_eq!(
        reg.list(&scope("")),
        Ok(vec![entry("gamma", "1.0.0", "any", &digest('9'))])
    );
}

#[test]
fn a_page_with_a_name_or_latest_mopm_would_not_accept_is_broken_and_no_record_is_read() {
    let broken = Err(SeamError::Permanent(
        "malformed answer: a page of packages".to_owned(),
    ));
    let cases = [
        ("Alpha", Some("1.0.0")),
        ("a", Some("1.0.0")),
        ("al-pha", Some("1.0.0")),
        ("alpha", Some("1.0")),
        ("alpha", Some("01.0.0")),
        ("alpha", Some("1.0.0-rc1")),
        ("alpha", Some("")),
    ];
    for bad in cases {
        let fake = Fake::default().on(
            Method::Get,
            SEARCH,
            vec![page(&[("bravo", Some("1.0.0")), bad], None)],
        );
        let fake = record_on(
            fake,
            "bravo",
            "1.0.0",
            json_reply(
                200,
                &record_json("bravo", "1.0.0", "published", &["linux-64"]),
            ),
        );
        let mut reg = registry(fake, &fresh());
        assert_eq!(reg.list(&scope("")), broken, "{bad:?}");
        assert_eq!(record_reads(&reg), 0, "{bad:?}");
    }
}

fn failures() -> Vec<(Reply, SeamError)> {
    let limited = |after: &str| {
        Ok(response(
            429,
            &[("retry-after", after)],
            &error_body("rate_limited"),
        ))
    };
    vec![
        (
            error_reply(401, "unauthorized"),
            SeamError::GrantRefused(GrantFault::Denied),
        ),
        (
            error_reply(403, "forbidden"),
            SeamError::GrantRefused(GrantFault::Denied),
        ),
        (
            error_reply(401, "token_expired"),
            SeamError::GrantRefused(GrantFault::Expired),
        ),
        (
            limited("600"),
            SeamError::RateLimited(Backoff { ticks: 600 }),
        ),
        (limited("601"), SeamError::Refused(Reason::Channel)),
        (limited("86400"), SeamError::Refused(Reason::Channel)),
    ]
}

#[test]
fn an_authorisation_or_rate_limit_failure_on_a_search_page_ends_the_listing() {
    for (reply, expected) in failures() {
        let fake = Fake::default().on(
            Method::Get,
            SEARCH,
            vec![
                page(&[("alpha", Some("1.0.0"))], Some("n")),
                reply.clone(),
                page(&[], None),
            ],
        );
        let fake = record_on(
            fake,
            "alpha",
            "1.0.0",
            json_reply(
                200,
                &record_json("alpha", "1.0.0", "published", &["linux-64"]),
            ),
        );
        let mut reg = registry(fake, &fresh());
        assert_eq!(reg.list(&scope("")), Err(expected.clone()), "{expected:?}");
        assert_eq!(searches(&reg), 2);
        assert_eq!(record_reads(&reg), 0);
    }
}

#[test]
fn an_authorisation_or_rate_limit_failure_on_a_record_read_ends_the_listing() {
    for (reply, expected) in failures() {
        let names = [("alpha", Some("1.0.0")), ("bravo", Some("1.0.0"))];
        let fake = Fake::default().on(Method::Get, SEARCH, vec![page(&names, None)]);
        let fake = record_on(fake, "alpha", "1.0.0", reply);
        let fake = record_on(
            fake,
            "bravo",
            "1.0.0",
            json_reply(
                200,
                &record_json("bravo", "1.0.0", "published", &["linux-64"]),
            ),
        );
        let mut reg = registry(fake, &fresh());
        assert_eq!(reg.list(&scope("")), Err(expected.clone()), "{expected:?}");
        assert_eq!(record_reads(&reg), 1);
        assert_eq!(
            reg.transport()
                .count(Method::Get, &record_path("bravo", "1.0.0")),
            0
        );
    }
}

#[test]
fn a_record_that_is_malformed_or_names_another_package_or_version_fails_the_listing() {
    let broken = Err(SeamError::Permanent(
        "malformed answer: a version record".to_owned(),
    ));
    let replies = [
        ok(200),
        text_reply(200, "{}"),
        json_reply(
            200,
            &record_json("bravo", "1.0.0", "published", &["linux-64"]),
        ),
        json_reply(
            200,
            &record_json("alpha", "1.0.1", "published", &["linux-64"]),
        ),
        json_reply(
            200,
            &record_json("Alpha", "1.0.0", "published", &["linux-64"]),
        ),
    ];
    for reply in replies {
        let names = [("alpha", Some("1.0.0")), ("bravo", Some("1.0.0"))];
        let fake = Fake::default().on(Method::Get, SEARCH, vec![page(&names, None)]);
        let fake = record_on(fake, "alpha", "1.0.0", reply.clone());
        let fake = record_on(
            fake,
            "bravo",
            "1.0.0",
            json_reply(
                200,
                &record_json("bravo", "1.0.0", "published", &["linux-64"]),
            ),
        );
        let mut reg = registry(fake, &fresh());
        assert_eq!(reg.list(&scope("")), broken, "{reply:?}");
        assert_eq!(record_reads(&reg), 1);
    }
}
