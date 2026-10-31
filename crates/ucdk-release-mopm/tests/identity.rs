// SPDX-License-Identifier: Apache-2.0
//! The identity token: its shape, its claims, what is compared with the declaration before it is
//! spent, and the GitHub Actions source of it.

mod common;
mod support;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use support::{NOW_MS, NOW_S, Reply, Script, claims, identity_token, jwt_of, raw};
use ucdk_release_mopm::{
    Expected, GITHUB_ISSUER, GithubIdentity, IdentitySource, IdentityToken, Method, Mismatch,
    TransportError, Url, check_identity,
};

fn pinned() -> BTreeMap<String, String> {
    BTreeMap::from([("ref".to_owned(), "refs/heads/main".to_owned())])
}

fn check(claims: &Value) -> Result<(String, String), Mismatch> {
    check_at(claims, NOW_MS)
}

fn check_at(claims: &Value, now_ms: u64) -> Result<(String, String), Mismatch> {
    let pinned = pinned();
    let token = identity_token(claims);
    let parsed = token.claims().unwrap();
    let expected = Expected {
        issuer: GITHUB_ISSUER,
        audience: support::AUDIENCE,
        subject: support::SUBJECT,
        environment: "publish",
        pinned: &pinned,
    };
    check_identity(&parsed, &expected, now_ms).map(|v| (v.jti, v.repository_id))
}

fn without(name: &str) -> Value {
    let mut c = claims("j1");
    c.as_object_mut().unwrap().remove(name);
    c
}

fn with(name: &str, value: Value) -> Value {
    let mut c = claims("j1");
    c[name] = value;
    c
}

#[test]
fn a_token_that_matches_the_declaration_passes_and_names_its_id_and_repository() {
    assert_eq!(
        check(&claims("j1")),
        Ok(("j1".to_owned(), "9001".to_owned()))
    );
}

#[test]
fn each_claim_that_differs_is_named() {
    let cases: Vec<(Value, Mismatch)> = vec![
        (with("iss", json!("https://evil.example")), Mismatch::Issuer),
        (without("iss"), Mismatch::Issuer),
        (with("aud", json!("other")), Mismatch::Audience),
        (without("aud"), Mismatch::Audience),
        (with("aud", json!(7)), Mismatch::Audience),
        (
            with("aud", json!([support::AUDIENCE, "extra"])),
            Mismatch::Audience,
        ),
        (
            with("aud", json!([support::AUDIENCE, 7])),
            Mismatch::Audience,
        ),
        (with("sub", json!("repo:other")), Mismatch::Subject),
        (without("sub"), Mismatch::Subject),
        (with("environment", json!("other")), Mismatch::Environment),
        (without("environment"), Mismatch::Environment),
        (
            with("ref", json!("refs/heads/dev")),
            Mismatch::Pinned("ref".to_owned()),
        ),
        (without("ref"), Mismatch::Pinned("ref".to_owned())),
        (with("ref", json!(1)), Mismatch::Pinned("ref".to_owned())),
        (
            with("repository_visibility", json!("private")),
            Mismatch::Visibility,
        ),
        (
            with("repository_visibility", json!("internal")),
            Mismatch::Visibility,
        ),
        (without("repository_visibility"), Mismatch::Visibility),
        (without("exp"), Mismatch::Expired),
        (with("exp", json!("soon")), Mismatch::Expired),
        (with("exp", json!(-1)), Mismatch::Expired),
        (with("exp", json!(NOW_S)), Mismatch::Expired),
        (with("exp", json!(NOW_S - 1)), Mismatch::Expired),
        (with("nbf", json!(NOW_S + 6)), Mismatch::NotYetValid),
        (with("nbf", json!("later")), Mismatch::NotYetValid),
        (without("iat"), Mismatch::IssuedAt),
        (with("iat", json!(NOW_S + 6)), Mismatch::IssuedAt),
        (with("iat", json!("now")), Mismatch::IssuedAt),
        (without("jti"), Mismatch::TokenId),
        (with("jti", json!("")), Mismatch::TokenId),
        (with("jti", json!(5)), Mismatch::TokenId),
        (without("repository_id"), Mismatch::Repository),
        (with("repository_id", json!("")), Mismatch::Repository),
    ];
    for (c, expected) in cases {
        assert_eq!(check(&c).err(), Some(expected), "{c}");
    }
}

#[test]
fn the_audience_may_be_a_list_of_exactly_the_declared_one() {
    assert!(check(&with("aud", json!([support::AUDIENCE]))).is_ok());
}

#[test]
fn an_expiry_is_never_excused_and_a_start_is_excused_by_the_skew_allowance_only() {
    // One second left is still good; none is not.
    assert!(check(&with("exp", json!(NOW_S + 1))).is_ok());
    assert_eq!(
        check_at(&with("exp", json!(NOW_S + 1)), NOW_MS + 1000).err(),
        Some(Mismatch::Expired)
    );
    // The allowance is five seconds.
    assert!(check(&with("nbf", json!(NOW_S + 5))).is_ok());
    assert!(check(&with("iat", json!(NOW_S + 5))).is_ok());
    assert!(check(&without("nbf")).is_ok());
}

#[test]
fn the_environment_may_be_absent_when_none_is_declared() {
    let pinned = pinned();
    let c = without("environment");
    let token = identity_token(&c);
    let parsed = token.claims().unwrap();
    let expected = Expected {
        issuer: GITHUB_ISSUER,
        audience: support::AUDIENCE,
        subject: support::SUBJECT,
        environment: "",
        pinned: &pinned,
    };
    assert!(check_identity(&parsed, &expected, NOW_MS).is_ok());
}

#[test]
fn a_token_is_three_base64url_parts_and_at_most_8192_bytes() {
    let payload = "e30"; // {}
    let ok = format!("e30.{payload}.sig").into_bytes();
    assert!(IdentityToken::new(ok).is_ok());
    for (what, bad) in [
        ("empty", String::new()),
        ("one part", "abc".to_owned()),
        ("two parts", "a.b".to_owned()),
        ("four parts", "a.b.c.d".to_owned()),
        ("a space", "a.b .c".to_owned()),
        ("a quote", "a.b\".c".to_owned()),
        ("padding", "a.b=.c".to_owned()),
        ("non-ascii", "a.b\u{e9}.c".to_owned()),
        ("too long", format!("a.{}.c", "b".repeat(8192))),
    ] {
        assert!(IdentityToken::new(bad.into_bytes()).is_err(), "{what}");
    }
    let longest = format!("a.{}.c", "b".repeat(8192 - 4));
    assert_eq!(longest.len(), 8192);
    assert!(IdentityToken::new(longest.into_bytes()).is_ok());
}

#[test]
fn a_payload_that_is_not_a_json_object_has_no_claims() {
    // base64url of: [1], "text", not json, and a valid object with a non-zero tail bit.
    for (what, payload) in [
        ("an array", "WzFd"),
        ("a string", "InRleHQi"),
        ("not json", "bm90IGpzb24"),
        ("empty", ""),
        ("one character", "e"),
        ("nonzero tail bits", "e31"),
        ("a lone sextet after a quad", "e30Ae"),
    ] {
        let token = IdentityToken::new(format!("h.{payload}.s").into_bytes()).unwrap();
        assert!(token.claims().is_err(), "{what}");
    }
}

#[test]
fn a_payload_of_every_length_is_read() {
    for len in 0..6 {
        let c = json!({ "k": "x".repeat(len) });
        let token = identity_token(&c);
        assert_eq!(
            token.claims().unwrap().text("k"),
            Some("x".repeat(len).as_str()),
            "{len}"
        );
    }
}

#[test]
fn claims_read_text_and_whole_seconds_only() {
    let token = identity_token(&json!({"a": "t", "n": 5, "f": 1.5, "neg": -1}));
    let c = token.claims().unwrap();
    assert_eq!(c.text("a"), Some("t"));
    assert_eq!(c.text("n"), None);
    assert_eq!(c.text("missing"), None);
    assert_eq!(c.seconds("n"), Some(5));
    assert_eq!(c.seconds("f"), None);
    assert_eq!(c.seconds("neg"), None);
    assert_eq!(c.seconds("a"), None);
    assert_eq!(c.seconds("missing"), None);
}

fn github(script: &Script) -> GithubIdentity<Script> {
    let url = Url::parse("https://pipelines.example.com/idtoken?api-version=2.0").unwrap();
    GithubIdentity::new(script.clone(), url, b"CANARY-request-token-77".to_vec()).unwrap()
}

#[test]
fn github_gives_a_token_for_the_audience_asked() {
    let script = Script::default();
    let jwt = support::token("j1");
    let body = json!({ "value": String::from_utf8(jwt.clone()).unwrap() }).to_string();
    script.push(raw(200, &[], &body));
    let mut source = github(&script);
    let token = source.identity_token("mopm.example.com").unwrap();
    assert_eq!(token.claims().unwrap().text("jti"), Some("j1"));
    let sent = script.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::Get);
    assert_eq!(
        sent[0].url,
        "https://pipelines.example.com/idtoken?api-version=2.0&audience=mopm.example.com"
    );
    assert_eq!(sent[0].headers, ["authorization", "accept"]);
    assert!(sent[0].body.is_empty());
}

#[test]
fn an_audience_is_percent_encoded_in_the_query() {
    let script = Script::default();
    script.push(raw(500, &[], ""));
    let url = Url::parse("https://pipelines.example.com/idtoken").unwrap();
    let mut source = GithubIdentity::new(script.clone(), url, b"t".to_vec()).unwrap();
    assert!(source.identity_token("a&b=c d").is_err());
    let sent = script.sent.lock().unwrap();
    assert_eq!(
        sent[0].url,
        "https://pipelines.example.com/idtoken?audience=a%26b%3Dc%20d"
    );
}

#[test]
fn a_request_token_that_cannot_be_a_header_is_refused() {
    let url = Url::parse("https://pipelines.example.com/idtoken").unwrap();
    for bad in [&b""[..], b"a b", b"a\nb"] {
        assert!(GithubIdentity::new(Script::default(), url.clone(), bad.to_vec()).is_none());
    }
}

#[test]
fn the_request_token_and_the_identity_token_never_reach_a_log() {
    let script = Script::default();
    let jwt = String::from_utf8(support::token("j1")).unwrap();
    script.push(raw(200, &[], &json!({ "value": jwt }).to_string()));
    let mut source = github(&script);
    let token = source.identity_token("aud").unwrap();
    drop(token);
    for line in script.debugs() {
        assert!(!line.contains("CANARY-request-token-77"), "{line}");
        assert!(!line.contains(&jwt), "{line}");
        assert!(line.contains("<redacted>"), "{line}");
    }
}

#[test]
fn github_gives_nothing_for_a_failure_or_an_answer_that_is_not_a_token() {
    let jwt = String::from_utf8(support::token("j1")).unwrap();
    let replies: Vec<(&str, Reply)> = vec![
        (
            "no connection",
            Reply::Raw(Err(TransportError::never_connected("x"))),
        ),
        ("401", raw(401, &[], &json!({ "value": jwt }).to_string())),
        ("not json", raw(200, &[], "<html>")),
        ("an array", raw(200, &[], "[]")),
        ("no value", raw(200, &[], "{}")),
        ("value not text", raw(200, &[], r#"{"value":5}"#)),
        (
            "value not a token",
            raw(200, &[], r#"{"value":"not a jwt"}"#),
        ),
    ];
    for (what, reply) in replies {
        let script = Script::default();
        script.push(reply);
        let mut source = github(&script);
        assert!(source.identity_token("aud").is_err(), "{what}");
    }
}

#[test]
fn a_token_gives_its_own_text_to_nothing_but_the_exchange_body() {
    // The only way to the bytes is the crate-private accessor used by the exchange; the claims
    // come from the payload and carry no signature.
    let token = IdentityToken::new(jwt_of(&claims("j9"))).unwrap();
    let c = token.claims().unwrap();
    assert_eq!(c.text("jti"), Some("j9"));
}

#[test]
fn an_unavailable_identity_says_why_and_nothing_else() {
    let error = ucdk_release_mopm::IdentityUnavailable("the platform gave no token");
    assert_eq!(
        error.to_string(),
        "no identity token: the platform gave no token"
    );
}
