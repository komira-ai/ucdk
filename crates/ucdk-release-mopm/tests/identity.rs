// SPDX-License-Identifier: Apache-2.0
//! The identity token: its shape, its claims, what is compared with the declaration before it is
//! spent, and the GitHub Actions source of it.

mod common;
mod support;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use support::{ISSUER, NOW_MS, NOW_S, Reply, Script, claims, identity_token, jwt_of, raw};
use ucdk_release_mopm::{
    Expected, GithubIdentity, IdentitySource, IdentityToken, IdentityUnavailable, Method, Mismatch,
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
        issuer: ISSUER,
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
        (
            with("iss", json!("https://evil.example.com")),
            Mismatch::Issuer,
        ),
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
        // Of the wrong type.
        (with("iss", json!(1)), Mismatch::Issuer),
        (with("iss", json!(null)), Mismatch::Issuer),
        (with("sub", json!([support::SUBJECT])), Mismatch::Subject),
        (
            with("repository_visibility", json!(true)),
            Mismatch::Visibility,
        ),
        (with("repository_id", json!(9001)), Mismatch::Repository),
        // An audience list that is not exactly the declared one.
        (with("aud", json!([])), Mismatch::Audience),
        (with("aud", json!(["other"])), Mismatch::Audience),
        (
            with("aud", json!([support::AUDIENCE, support::AUDIENCE])),
            Mismatch::Audience,
        ),
        (
            with("aud", json!({"a": support::AUDIENCE})),
            Mismatch::Audience,
        ),
        // A present environment that is not text, when one is declared.
        (with("environment", json!(null)), Mismatch::Environment),
        (
            with("environment", json!(["publish"])),
            Mismatch::Environment,
        ),
        // Dates that are not whole non-negative seconds that fit.
        (with("exp", json!(NOW_S as f64 + 300.5)), Mismatch::Expired),
        (with("exp", json!(1e20)), Mismatch::Expired),
        (with("iat", json!(NOW_S as f64 - 0.5)), Mismatch::IssuedAt),
        (with("nbf", json!(-1)), Mismatch::NotYetValid),
        (with("nbf", json!(u64::MAX)), Mismatch::NotYetValid),
        (with("iat", json!(u64::MAX)), Mismatch::IssuedAt),
        // A repository id that is not decimal digits with no leading zero: it names the actor.
        (with("repository_id", json!("0")), Mismatch::Repository),
        (with("repository_id", json!("09001")), Mismatch::Repository),
        (with("repository_id", json!("9001:x")), Mismatch::Repository),
        (with("repository_id", json!("-9001")), Mismatch::Repository),
        (with("repository_id", json!("9 001")), Mismatch::Repository),
        (
            with("repository_id", json!("\u{661}")),
            Mismatch::Repository,
        ),
    ];
    for (c, expected) in cases {
        assert_eq!(check(&c).err(), Some(expected), "{c}");
    }
}

#[test]
fn a_date_that_does_not_fit_in_milliseconds_is_read_as_the_latest_instant() {
    // `exp` of the largest whole number of seconds is far in the future, not wrapped round.
    assert!(check(&with("exp", json!(u64::MAX))).is_ok());
}

#[test]
fn every_pinned_claim_is_checked_on_its_own() {
    let pinned = BTreeMap::from([
        (
            "job_workflow_ref".to_owned(),
            "example-org/lib/.github/workflows/release.yml@refs/heads/main".to_owned(),
        ),
        ("ref".to_owned(), "refs/heads/main".to_owned()),
        ("repository_owner".to_owned(), "example-org".to_owned()),
    ]);
    let mut good = claims("j1");
    for (name, value) in &pinned {
        good[name] = json!(value);
    }
    let run = |c: &Value| {
        let token = identity_token(c);
        let parsed = token.claims().unwrap();
        let expected = Expected {
            issuer: ISSUER,
            audience: support::AUDIENCE,
            subject: support::SUBJECT,
            environment: "publish",
            pinned: &pinned,
        };
        check_identity(&parsed, &expected, NOW_MS).err()
    };
    assert_eq!(run(&good), None);
    for name in pinned.keys() {
        let mut changed = good.clone();
        changed[name] = json!("other");
        assert_eq!(
            run(&changed),
            Some(Mismatch::Pinned(name.clone())),
            "{name}"
        );
        let mut removed = good.clone();
        removed.as_object_mut().unwrap().remove(name);
        assert_eq!(
            run(&removed),
            Some(Mismatch::Pinned(name.clone())),
            "{name}"
        );
    }
}

#[test]
fn a_mismatch_names_the_claim_and_nothing_of_its_value() {
    let cases = [
        (Mismatch::Issuer, "iss"),
        (Mismatch::Audience, "aud"),
        (Mismatch::Subject, "sub"),
        (Mismatch::Environment, "environment"),
        (Mismatch::Pinned("ref".to_owned()), "ref"),
        (Mismatch::Visibility, "repository_visibility"),
        (Mismatch::Expired, "exp"),
        (Mismatch::NotYetValid, "nbf"),
        (Mismatch::IssuedAt, "iat"),
        (Mismatch::TokenId, "jti"),
        (Mismatch::Repository, "repository_id"),
    ];
    for (mismatch, claim) in cases {
        assert_eq!(
            mismatch.to_string(),
            format!("the identity token's {claim} claim is not what the declaration admits")
        );
        let error: &dyn std::error::Error = &mismatch;
        assert!(error.source().is_none());
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
    // To the millisecond: the last millisecond before `exp` is good, `exp` itself is not.
    let exp = with("exp", json!(NOW_S + 1));
    assert!(check_at(&exp, NOW_MS + 999).is_ok());
    assert_eq!(check_at(&exp, NOW_MS + 1000).err(), Some(Mismatch::Expired));
    // A start exactly at now plus the allowance is good, one millisecond further is not.
    let start = NOW_S + 10;
    for (name, mismatch) in [("nbf", Mismatch::NotYetValid), ("iat", Mismatch::IssuedAt)] {
        let c = with(name, json!(start));
        assert!(check_at(&c, start * 1000 - 5000).is_ok(), "{name}");
        assert_eq!(
            check_at(&c, start * 1000 - 5001).err(),
            Some(mismatch),
            "{name}"
        );
    }
}

fn with_no_environment_declared(c: &Value) -> Result<(), Mismatch> {
    let pinned = pinned();
    let token = identity_token(c);
    let parsed = token.claims().unwrap();
    let expected = Expected {
        issuer: ISSUER,
        audience: support::AUDIENCE,
        subject: support::SUBJECT,
        environment: "",
        pinned: &pinned,
    };
    check_identity(&parsed, &expected, NOW_MS).map(|_| ())
}

#[test]
fn the_environment_may_be_absent_when_none_is_declared() {
    assert_eq!(
        with_no_environment_declared(&without("environment")),
        Ok(())
    );
    assert_eq!(
        with_no_environment_declared(&with("environment", json!(""))),
        Ok(())
    );
}

#[test]
fn an_environment_that_is_present_and_not_text_is_not_an_absent_one() {
    for value in [json!(null), json!(0), json!(false), json!([]), json!({})] {
        assert_eq!(
            with_no_environment_declared(&with("environment", value.clone())),
            Err(Mismatch::Environment),
            "{value}"
        );
    }
    assert_eq!(
        with_no_environment_declared(&with("environment", json!("publish"))),
        Err(Mismatch::Environment)
    );
}

#[test]
fn a_token_is_three_non_empty_base64url_parts_and_at_most_8192_bytes() {
    // `e30` is `{}`, and `c2ln` is `sig`.
    assert!(IdentityToken::new(b"e30.e30.c2ln".to_vec()).is_ok());
    for (what, bad) in [
        ("empty", String::new()),
        ("one part", "e30".to_owned()),
        ("two parts", "e30.e30".to_owned()),
        ("four parts", "e30.e30.e30.e30".to_owned()),
        ("empty header", ".e30.c2ln".to_owned()),
        ("empty payload", "e30..c2ln".to_owned()),
        ("empty signature", "e30.e30.".to_owned()),
        ("only dots", "..".to_owned()),
        ("a space", "e30.e3 0.c2ln".to_owned()),
        ("a quote", "e30.e30\".c2ln".to_owned()),
        ("padding", "e30.e30=.c2ln".to_owned()),
        ("standard alphabet", "e30.e30.c2+n".to_owned()),
        ("non-ascii", "e30.e3\u{e9}0.c2ln".to_owned()),
        ("a lone character in the header", "e.e30.c2ln".to_owned()),
        ("a lone character in the signature", "e30.e30.c".to_owned()),
        ("nonzero tail bits in the header", "e31.e30.c2ln".to_owned()),
        (
            "nonzero tail bits in the signature",
            "e30.e30.c2m".to_owned(),
        ),
        ("a lone character in the payload", "e30.e.c2ln".to_owned()),
        (
            "nonzero tail bits in the payload",
            "e30.e31.c2ln".to_owned(),
        ),
        ("a lone sextet after a quad", "e30.e30Ae.c2ln".to_owned()),
        ("too long", format!("e30.{}.e30", "A".repeat(8188))),
    ] {
        assert_eq!(
            IdentityToken::new(bad.into_bytes())
                .err()
                .map(|e| e.to_string()),
            Some("no identity token: the token is not a JWT".to_owned()),
            "{what}"
        );
    }
    let longest = format!("e30.{}.e30", "A".repeat(8192 - 8));
    assert_eq!(longest.len(), 8192);
    assert!(IdentityToken::new(longest.into_bytes()).is_ok());
}

#[test]
fn a_payload_that_is_not_one_json_object_has_no_claims() {
    // base64url of: [1], "text", not json, and an object followed by more text.
    for (what, payload) in [
        ("an array", "WzFd"),
        ("a string", "InRleHQi"),
        ("not json", "bm90IGpzb24"),
        ("an object and more", "e30gMQ"),
    ] {
        let token = IdentityToken::new(format!("e30.{payload}.c2ln").into_bytes()).unwrap();
        assert_eq!(
            token.claims().err(),
            Some(IdentityUnavailable::PayloadNotAnObject),
            "{what}"
        );
    }
}

#[test]
fn a_payload_that_names_a_claim_twice_has_no_claims() {
    // Two readers could take different values: neither is taken.
    for name in [
        "iss",
        "aud",
        "sub",
        "environment",
        "exp",
        "jti",
        "repository_id",
        "ref",
    ] {
        let good = claims("j1");
        let mut text = good.to_string();
        // The repeat comes last, so a reader that keeps the last value would see it.
        text.insert_str(text.len() - 1, &format!(r#","{name}":"other""#));
        let payload = support::b64(text.as_bytes());
        let token = IdentityToken::new(format!("e30.{payload}.c2ln").into_bytes()).unwrap();
        assert_eq!(
            token.claims().err(),
            Some(IdentityUnavailable::RepeatedClaim),
            "{name}"
        );
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

const REQUEST_TOKEN: &[u8] = b"CANARY-request-token-77";

fn github(script: &Script) -> GithubIdentity<Script> {
    let url = Url::parse("https://pipelines.example.com/idtoken?api-version=2.0").unwrap();
    GithubIdentity::new(script.clone(), url, REQUEST_TOKEN.to_vec()).unwrap()
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

/// The URL the platform is asked, from the request URL the job is given and an audience.
fn asked(request_url: &str, audience: &str) -> String {
    let script = Script::default();
    script.push(raw(500, &[], ""));
    let url = Url::parse(request_url).unwrap();
    let mut source = GithubIdentity::new(script.clone(), url, b"t".to_vec()).unwrap();
    assert_eq!(
        source.identity_token(audience).err(),
        Some(IdentityUnavailable::NoToken)
    );
    script.sent.lock().unwrap()[0].url.clone()
}

#[test]
fn an_audience_is_percent_encoded_in_the_query() {
    let base = "https://pipelines.example.com/idtoken";
    let cases = [
        ("a&b=c d", "audience=a%26b%3Dc%20d"),
        ("", "audience="),
        ("caf\u{e9}", "audience=caf%C3%A9"),
        ("a/b?c#d+e%", "audience=a%2Fb%3Fc%23d%2Be%25"),
    ];
    for (audience, query) in cases {
        assert_eq!(
            asked(base, audience),
            format!("{base}?{query}"),
            "{audience}"
        );
    }
}

#[test]
fn an_audience_already_in_the_request_url_is_replaced_and_every_other_parameter_kept() {
    let base = "https://pipelines.example.com/idtoken";
    let cases = [
        ("?audience=old", "?audience=new"),
        ("?audience=old&audience=older", "?audience=new"),
        ("?audience", "?audience=new"),
        ("?a=1&audience=old&b=2", "?a=1&b=2&audience=new"),
        (
            "?audiences=x&xaudience=y",
            "?audiences=x&xaudience=y&audience=new",
        ),
        ("?", "?audience=new"),
        ("?a=1&&b=2", "?a=1&b=2&audience=new"),
        ("?api-version=2.0", "?api-version=2.0&audience=new"),
    ];
    for (query, expected) in cases {
        assert_eq!(
            asked(&format!("{base}{query}"), "new"),
            format!("{base}{expected}"),
            "{query}"
        );
    }
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
    assert_eq!(
        script.debugs(),
        [concat!(
            r#"Request { method: "GET", url: "https://pipelines.example.com/idtoken?<redacted>", "#,
            r#"headers: ["authorization: <redacted>", "accept: application/json"], body_len: 0 }"#
        )]
    );
    let request_token = String::from_utf8(REQUEST_TOKEN.to_vec()).unwrap();
    for line in script.debugs() {
        assert!(!line.contains(&request_token), "{line}");
        assert!(!line.contains(&jwt), "{line}");
    }
}

#[test]
fn github_gives_nothing_for_a_failure_or_an_answer_that_is_not_a_token() {
    let jwt = String::from_utf8(support::token("j1")).unwrap();
    use IdentityUnavailable::{NoToken, NotAJwt, Unreachable};
    let replies: Vec<(&str, Reply, IdentityUnavailable)> = vec![
        (
            "no connection",
            Reply::Raw(Err(TransportError::never_connected("x"))),
            Unreachable,
        ),
        (
            "401",
            raw(401, &[], &json!({ "value": jwt }).to_string()),
            NoToken,
        ),
        ("not json", raw(200, &[], "<html>"), NoToken),
        ("an array", raw(200, &[], "[]"), NoToken),
        ("no value", raw(200, &[], "{}"), NoToken),
        ("value not text", raw(200, &[], r#"{"value":5}"#), NoToken),
        (
            "value repeated",
            raw(200, &[], &format!(r#"{{"value":"{jwt}","value":"{jwt}"}}"#)),
            NoToken,
        ),
        (
            "another field repeated",
            raw(
                200,
                &[],
                &format!(r#"{{"value":"{jwt}","count":1,"count":2}}"#),
            ),
            NoToken,
        ),
        (
            "an object and more",
            raw(200, &[], &format!(r#"{{"value":"{jwt}"}} x"#)),
            NoToken,
        ),
        (
            "value not a token",
            raw(200, &[], r#"{"value":"not a jwt"}"#),
            NotAJwt,
        ),
    ];
    for (what, reply, why) in replies {
        let script = Script::default();
        script.push(reply);
        let mut source = github(&script);
        assert_eq!(source.identity_token("aud").err(), Some(why), "{what}");
    }
    // Fields it does not know are ignored, and an escaped character is read.
    let script = Script::default();
    let escaped = jwt.replacen('.', "\\u002e", 1);
    script.push(raw(
        200,
        &[],
        &format!(r#"{{"count":1,"value":"{escaped}"}}"#),
    ));
    let token = github(&script).identity_token("aud").unwrap();
    assert_eq!(token.claims().unwrap().text("jti"), Some("j1"));
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
    let cases = [
        (IdentityUnavailable::NotAJwt, "the token is not a JWT"),
        (
            IdentityUnavailable::PayloadNotAnObject,
            "the payload is not a JSON object",
        ),
        (
            IdentityUnavailable::RepeatedClaim,
            "the payload names a claim twice",
        ),
        (
            IdentityUnavailable::Unreachable,
            "the platform could not be reached",
        ),
        (IdentityUnavailable::NoToken, "the platform gave no token"),
    ];
    for (error, why) in cases {
        assert_eq!(error.to_string(), format!("no identity token: {why}"));
        let error: &dyn std::error::Error = &error;
        assert!(error.source().is_none());
    }
}
