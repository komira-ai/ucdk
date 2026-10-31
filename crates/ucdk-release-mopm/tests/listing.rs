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
    let body = bytes(&json!({"versions": [
        {"version": "1.0.1", "state": "published"},
        {"version": "1.0.0", "state": "yanked"},
        {"version": "0.9.0", "state": "tombstoned"},
    ]}));
    assert_eq!(
        parse_package(&body),
        Some(vec![
            ("1.0.1".to_owned(), VersionState::Published),
            ("1.0.0".to_owned(), VersionState::Yanked),
            ("0.9.0".to_owned(), VersionState::Tombstoned),
        ])
    );
    assert_eq!(
        parse_package(&bytes(&json!({"versions": []}))),
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
        assert_eq!(parse_package(&bytes(&bad)), None, "{bad}");
    }
    assert_eq!(parse_package(b"not json"), None);
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
