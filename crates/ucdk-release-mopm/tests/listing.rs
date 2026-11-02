// SPDX-License-Identifier: Apache-2.0
//! The readers of the bodies that list, the search prefix, and the two ways to build a location
//! that the reads need.

mod common;

use common::{DIGEST, index_body, record_json};
use serde_json::{Value, json};
use ucdk_release_mopm::{
    Page, Url, VersionState, parse_index, parse_package, parse_page, valid_prefix,
};

fn bytes(value: &Value) -> Vec<u8> {
    value.to_string().into_bytes()
}

#[test]
fn a_package_record_lists_its_versions_with_their_states() {
    let body = bytes(&json!({"name": "alpha", "versions": [
        {"version": "1.0.1", "state": "published"},
        {"version": "1.0.0", "state": "yanked"},
        {"version": "0.9.0", "state": "tombstoned"},
    ]}));
    assert_eq!(
        parse_package(&body, "alpha"),
        Some(vec![
            ("1.0.1".to_owned(), VersionState::Published),
            ("1.0.0".to_owned(), VersionState::Yanked),
            ("0.9.0".to_owned(), VersionState::Tombstoned),
        ])
    );
    assert_eq!(
        parse_package(&bytes(&json!({"name": "alpha", "versions": []})), "alpha"),
        Some(vec![])
    );
    for bad in [
        json!({}),
        json!({"versions": "none"}),
        json!({"versions": [{"version": "1.0.0"}]}),
        json!({"versions": [{"state": "published"}]}),
        json!({"versions": [{"version": "1.0.0", "state": "pending"}]}),
        json!({"versions": [{"version": 1, "state": "published"}]}),
    ] {
        assert_eq!(parse_package(&bytes(&bad), "alpha"), None, "{bad}");
    }
    assert_eq!(parse_package(b"not json", "alpha"), None);
}

#[test]
fn a_page_of_packages_has_names_latest_versions_and_a_cursor() {
    let body = bytes(&json!({
        "packages": [
            {"name": "alpha", "latest": "1.0.0", "description": null},
            {"name": "beta", "latest": null},
        ],
        "next_cursor": "c1",
    }));
    assert_eq!(
        parse_page(&body),
        Some(Page {
            packages: vec![
                ("alpha".to_owned(), Some("1.0.0".to_owned())),
                ("beta".to_owned(), None),
            ],
            next_cursor: Some("c1".to_owned()),
        })
    );
    let last = bytes(&json!({"packages": [], "next_cursor": null}));
    assert_eq!(
        parse_page(&last),
        Some(Page {
            packages: vec![],
            next_cursor: None,
        })
    );
    for bad in [
        json!({"packages": []}),
        json!({"packages": [], "next_cursor": 1}),
        json!({"next_cursor": null}),
        json!({"packages": "none", "next_cursor": null}),
        json!({"packages": [{"latest": "1.0.0"}], "next_cursor": null}),
        json!({"packages": [{"name": "a"}], "next_cursor": null}),
        json!({"packages": [{"name": "a", "latest": 1}], "next_cursor": null}),
    ] {
        assert_eq!(parse_page(&bytes(&bad)), None, "{bad}");
    }
    assert_eq!(parse_page(b"[]"), None);
}

#[test]
fn a_sparse_index_file_is_read_whole_or_not_at_all() {
    let one = record_json("alpha", "1.0.0", "published", &["linux-64"]);
    let two = record_json("alpha", "1.0.1", "yanked", &["linux-64", "noarch"]);
    let body = index_body(&[one.clone(), two]);
    let records = parse_index(body.as_bytes(), "alpha").unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].state, VersionState::Yanked);
    assert_eq!(records[0].files[0].sha256, DIGEST);
    // An unknown key is ignored.
    let mut extra = one.clone();
    extra["future"] = json!(true);
    assert!(parse_index(index_body(&[extra]).as_bytes(), "alpha").is_some());
    let truncated = body.trim_end();
    let mut newer = one.clone();
    newer["v"] = json!(2);
    for bad in [
        String::new(),
        truncated.to_owned(),
        format!("{newer}\n"),
        format!("{one}\n"),
        index_body(&[one.clone(), one.clone()]),
        format!("{body}\n"),
        "not json\n".to_owned(),
    ] {
        assert!(parse_index(bad.as_bytes(), "alpha").is_none(), "{bad:?}");
    }
    assert!(parse_index(body.as_bytes(), "beta").is_none());
    assert!(parse_index(&[0xff, b'\n'], "alpha").is_none());
}

#[test]
fn a_search_prefix_is_at_most_64_lower_case_name_characters() {
    assert!(valid_prefix(""));
    assert!(valid_prefix("al"));
    assert!(valid_prefix("http_client2"));
    assert!(valid_prefix(&"a".repeat(64)));
    assert!(!valid_prefix(&"a".repeat(65)));
    for bad in ["Al", "a-b", "a b", "a/b", "\u{e9}"] {
        assert!(!valid_prefix(bad), "{bad}");
    }
}

#[test]
fn a_url_can_take_a_query_and_can_be_rooted_at_its_origin() {
    let api = Url::parse("https://registry.example.com/api/v1").unwrap();
    let search = api.join(&["packages"]).with_query("sort=name&q=al");
    assert!(search.has_query());
    assert_eq!(search.path(), "/api/v1/packages");
    assert_eq!(
        search.expose(),
        "https://registry.example.com/api/v1/packages?sort=name&q=al"
    );
    assert_eq!(
        search.redacted(),
        "https://registry.example.com/api/v1/packages?<redacted>"
    );
    // The index is not under the API's path, and a segment cannot end itself.
    let index = api.rooted(&["index", "al", "ph", "a/b"]);
    assert_eq!(index.path(), "/index/al/ph/a%2Fb");
    assert!(!index.has_query());
    assert_eq!(index.origin(), api.origin());
}

#[test]
fn valid_entries_followed_by_a_malformed_one_make_the_whole_body_unreadable() {
    let package = bytes(&json!({"name": "alpha", "versions": [
        {"version": "1.0.1", "state": "published"},
        {"version": "1.0.0", "state": "yanked"},
        {"version": "0.9.0"},
    ]}));
    assert_eq!(parse_package(&package, "alpha"), None);
    let page = bytes(&json!({
        "packages": [
            {"name": "alpha", "latest": "1.0.0"},
            {"name": "beta", "latest": null},
            {"name": "gamma"},
        ],
        "next_cursor": null,
    }));
    assert_eq!(parse_page(&page), None);
    let one = record_json("alpha", "1.0.0", "published", &["linux-64"]);
    let two = record_json("alpha", "1.0.1", "yanked", &["linux-64"]);
    let mut broken = record_json("alpha", "1.0.2", "published", &["linux-64"]);
    broken["files"] = json!("none");
    let body = index_body(&[one.clone(), two.clone(), broken]);
    assert!(parse_index(body.as_bytes(), "alpha").is_none());
    // The same body without its last line reads whole.
    let good = index_body(&[one, two]);
    assert_eq!(parse_index(good.as_bytes(), "alpha").unwrap().len(), 2);
}

#[test]
fn unknown_fields_at_every_level_of_a_listing_body_change_nothing() {
    let plain = bytes(&json!({"name": "alpha", "versions": [
        {"version": "1.0.1", "state": "published"},
        {"version": "1.0.0", "state": "yanked"},
    ]}));
    let extra = bytes(&json!({"name": "alpha", "future": [1], "versions": [
        {"version": "1.0.1", "state": "published", "added": {"x": 1}},
        {"version": "1.0.0", "state": "yanked", "reason": "r"},
    ]}));
    assert_eq!(
        parse_package(&extra, "alpha"),
        parse_package(&plain, "alpha")
    );
    assert_eq!(
        parse_package(&extra, "alpha"),
        Some(vec![
            ("1.0.1".to_owned(), VersionState::Published),
            ("1.0.0".to_owned(), VersionState::Yanked),
        ])
    );
    let plain = bytes(&json!({
        "packages": [{"name": "alpha", "latest": "1.0.0"}, {"name": "beta", "latest": null}],
        "next_cursor": "c1",
    }));
    let extra = bytes(&json!({
        "total": 2,
        "packages": [
            {"name": "alpha", "latest": "1.0.0", "downloads": 5},
            {"name": "beta", "latest": null, "owner": {"id": 1}},
        ],
        "next_cursor": "c1",
    }));
    assert_eq!(parse_page(&extra), parse_page(&plain));
    assert_eq!(
        parse_page(&extra),
        Some(Page {
            packages: vec![
                ("alpha".to_owned(), Some("1.0.0".to_owned())),
                ("beta".to_owned(), None),
            ],
            next_cursor: Some("c1".to_owned()),
        })
    );
}

#[test]
fn a_package_record_is_refused_for_another_name_a_bad_version_or_a_repeat() {
    let listing = |name: Option<&str>, versions: &[&str]| {
        let versions: Vec<Value> = versions
            .iter()
            .map(|v| json!({"version": v, "state": "published"}))
            .collect();
        let mut body = json!({"versions": versions});
        if let Some(name) = name {
            body["name"] = json!(name);
        }
        bytes(&body)
    };
    assert_eq!(
        parse_package(&listing(Some("alpha"), &["1.0.0"]), "alpha"),
        Some(vec![("1.0.0".to_owned(), VersionState::Published)])
    );
    assert_eq!(parse_package(&listing(None, &["1.0.0"]), "alpha"), None);
    assert_eq!(
        parse_package(&listing(Some("beta"), &["1.0.0"]), "alpha"),
        None
    );
    assert_eq!(
        parse_package(&bytes(&json!({"name": 1, "versions": []})), "alpha"),
        None
    );
    assert_eq!(
        parse_package(
            &listing(Some("alpha"), &["1.0.0", "1.0.1", "1.0.0"]),
            "alpha"
        ),
        None
    );
    let longest = format!("1.0.{}", "9".repeat(128 - 4));
    let too_long = format!("1.0.{}", "9".repeat(128 - 3));
    assert_eq!(longest.len(), 128);
    assert_eq!(
        parse_package(&listing(Some("alpha"), &[&longest]), "alpha"),
        Some(vec![(longest.clone(), VersionState::Published)])
    );
    for bad in [
        "",
        "1.0 0",
        "1.0",
        "1.0.0.1.5",
        "01.0.0",
        "1.0.x",
        "v1.0.0",
        "1.0.0-rc1",
        too_long.as_str(),
    ] {
        assert_eq!(
            parse_package(&listing(Some("alpha"), &["0.9.0", bad]), "alpha"),
            None,
            "{bad:?}"
        );
    }
}

#[test]
fn a_page_is_refused_for_a_bad_name_a_bad_latest_version_or_an_empty_cursor() {
    let page = |package: Value, cursor: Value| {
        bytes(&json!({"packages": [package], "next_cursor": cursor}))
    };
    assert_eq!(
        parse_page(&page(
            json!({"name": "alpha_2", "latest": "10.20.30"}),
            json!("c")
        )),
        Some(Page {
            packages: vec![("alpha_2".to_owned(), Some("10.20.30".to_owned()))],
            next_cursor: Some("c".to_owned()),
        })
    );
    for name in [
        "Alpha",
        "",
        "a",
        "1alpha",
        "al-pha",
        "al pha",
        &"a".repeat(65),
    ] {
        assert_eq!(
            parse_page(&page(json!({"name": name, "latest": null}), json!(null))),
            None,
            "{name:?}"
        );
    }
    for latest in [
        "",
        "1.0",
        "1.0 0",
        "01.0.0",
        "1.0.0-rc1",
        &format!("1.0.{}", "9".repeat(125)),
    ] {
        assert_eq!(
            parse_page(&page(
                json!({"name": "alpha", "latest": latest}),
                json!(null)
            )),
            None,
            "{latest:?}"
        );
    }
    assert_eq!(
        parse_page(&page(json!({"name": "alpha", "latest": null}), json!(""))),
        None
    );
}

#[test]
fn a_query_on_a_parsed_url_is_replaced_by_with_query() {
    let url = Url::parse("https://registry.example.com/api/v1/packages?old=1").unwrap();
    assert!(url.has_query());
    assert_eq!(url.path(), "/api/v1/packages");
    let replaced = url.with_query("new=2");
    assert!(replaced.has_query());
    assert_eq!(replaced.path(), "/api/v1/packages");
    assert_eq!(
        replaced.expose(),
        "https://registry.example.com/api/v1/packages?new=2"
    );
    assert_eq!(
        replaced.redacted(),
        "https://registry.example.com/api/v1/packages?<redacted>"
    );
}

#[test]
fn an_empty_query_means_no_query() {
    let url = Url::parse("https://registry.example.com/api/v1/packages?old=1").unwrap();
    let bare = url.with_query("");
    assert!(!bare.has_query());
    assert_eq!(
        bare.expose(),
        "https://registry.example.com/api/v1/packages"
    );
    assert_eq!(
        bare.redacted(),
        "https://registry.example.com/api/v1/packages"
    );
    let never = Url::parse("https://registry.example.com/api/v1/packages").unwrap();
    assert!(!never.with_query("").has_query());
    assert!(bare == never);
}

#[test]
fn rooting_a_url_drops_its_query_and_an_empty_root_is_the_slash_path() {
    let url = Url::parse("https://registry.example.com:8443/api/v1?sig=secret").unwrap();
    let index = url.rooted(&["index", "al"]);
    assert!(!index.has_query());
    assert_eq!(index.path(), "/index/al");
    assert_eq!(index.expose(), "https://registry.example.com:8443/index/al");
    assert_eq!(
        index.redacted(),
        "https://registry.example.com:8443/index/al"
    );
    let root = url.rooted(&[]);
    assert!(!root.has_query());
    assert_eq!(root.path(), "/");
    assert_eq!(root.expose(), "https://registry.example.com:8443/");
    assert_eq!(root.redacted(), "https://registry.example.com:8443/");
}

#[test]
fn an_empty_segment_in_a_rooted_path_stays_as_a_trailing_slash() {
    // The empty segment is kept, not skipped: "index" followed by "" is "/index/".
    let url = Url::parse("https://host.example.com/api").unwrap();
    let rooted = url.rooted(&["index", ""]);
    assert_eq!(rooted.path(), "/index/");
    assert_eq!(rooted.expose(), "https://host.example.com/index/");
}
