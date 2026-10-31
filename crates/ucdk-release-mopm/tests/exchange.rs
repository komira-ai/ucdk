// SPDX-License-Identifier: Apache-2.0
//! The mopm grant source: what it checks before it asks for an identity token, what it sends,
//! how it reads the answer, and that no credential reaches an error, a log or a second request.

mod common;
mod support;

use common::coordinate;
use serde_json::{Value, json};
use support::{
    LOCATION, MINTED, NOW_MS, NOW_S, Reply, claims, echo, iso, jwt_of, raw, repository,
    repository_with, request, source, source_for, token,
};
use ucdk_ledger::ManualClock;
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_mopm::{ActorForm, Envelope, Method, TransportError, Use, Verb, authorise};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{Anonymous, Coordinate, PushIdentity, Reason, Visibility};
use ucdk_release_seams::{Backoff, Grant, GrantRequest, GrantSource, SeamError};

fn pair() -> Vec<Coordinate> {
    vec![
        coordinate("alpha", "1.2.3", "linux-64"),
        coordinate("alpha", "1.2.3", "osx-arm64"),
        coordinate("beta", "0.1.0", "noarch"),
    ]
}

fn denied() -> SeamError {
    SeamError::GrantRefused(GrantFault::Denied)
}

fn refused(fault: GrantFault) -> SeamError {
    SeamError::GrantRefused(fault)
}

fn texts(error: &SeamError) -> String {
    format!("{error:?} / {error}")
}

fn decoded(grant: &Grant) -> Envelope {
    Envelope::decode(grant.secret()).unwrap()
}

#[test]
fn a_stage_grant_exchanges_one_identity_token_for_the_distinct_pairs() {
    let (mut source, rig) = source();
    let jwt = token("j1");
    rig.tokens.push(jwt.clone());
    rig.script.push(echo(&iso(600)));
    let repo = repository();
    let set = pair();
    let grant = source
        .grant(&request(&repo, GrantPhase::Stage, &set))
        .unwrap();

    assert_eq!(*rig.tokens.audiences.lock().unwrap(), [support::AUDIENCE]);
    let sent = rig.script.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::Post);
    assert_eq!(
        sent[0].url,
        "https://registry.example.com/api/v1/auth/oidc/exchange"
    );
    assert_eq!(sent[0].headers, ["content-type", "accept"]);
    let body: Value = serde_json::from_slice(&sent[0].body).unwrap();
    assert_eq!(body["jwt"].as_str().unwrap().as_bytes(), jwt.as_slice());
    assert_eq!(
        body["packages"],
        json!([
            {"name": "alpha", "version": "1.2.3"},
            {"name": "beta", "version": "0.1.0"},
        ])
    );
    assert_eq!(body.as_object().unwrap().len(), 2);

    let envelope = decoded(&grant);
    assert_eq!(envelope.phase, GrantPhase::Stage);
    assert!(!envelope.exceptional);
    assert_eq!(envelope.registry, LOCATION);
    assert_eq!(envelope.actor, "ci:gh-repo:9001");
    assert_eq!(envelope.coordinates, set);
    assert_eq!(envelope.bearer_token(), Some(MINTED.as_bytes()));
    assert_eq!(grant.expires_ms(), Some(NOW_MS + 600_000));
}

#[test]
fn the_commit_grant_is_a_second_exchange_with_a_new_identity_token() {
    let (mut source, rig) = source();
    let repo = repository();
    let set = pair();
    for (jti, phase) in [("j1", GrantPhase::Stage), ("j2", GrantPhase::Commit)] {
        rig.tokens.push(token(jti));
        rig.script.push(echo(&iso(600)));
        let grant = source.grant(&request(&repo, phase, &set)).unwrap();
        assert_eq!(decoded(&grant).phase, phase);
    }
    let sent = rig.script.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    let jwts: Vec<String> = sent
        .iter()
        .map(|s| {
            let body: Value = serde_json::from_slice(&s.body).unwrap();
            body["jwt"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_ne!(jwts[0], jwts[1]);
    assert_eq!(rig.tokens.asked(), 2);
}

#[test]
fn the_actor_is_the_account_when_the_identity_acts_for_one() {
    let (mut source, rig) = source_for(ActorForm::Account(42), &ManualClock::at(NOW_MS));
    rig.tokens.push(token("j1"));
    rig.script.push(echo(&iso(600)));
    let repo = repository();
    let set = pair();
    let grant = source
        .grant(&request(&repo, GrantPhase::Stage, &set))
        .unwrap();
    assert_eq!(decoded(&grant).actor, "u:42");
}

#[test]
fn an_exceptional_run_presents_the_exceptional_subject_and_environment() {
    let (mut source, rig) = source();
    let repo = repository();
    let set = pair();
    let mut exceptional = claims("j1");
    exceptional["sub"] = json!("repo:example-org/lib:environment:exceptional");
    exceptional["environment"] = json!("exceptional");
    rig.tokens.push(jwt_of(&exceptional));
    rig.script.push(echo(&iso(600)));
    let req = GrantRequest {
        exceptional: true,
        ..request(&repo, GrantPhase::Stage, &set)
    };
    let grant = source.grant(&req).unwrap();
    assert!(decoded(&grant).exceptional);

    // The ordinary subject does not pass for an exceptional run, and the reverse.
    rig.tokens.push(token("j2"));
    assert_eq!(source.grant(&req).err(), Some(denied()));
    rig.tokens.push(jwt_of(&exceptional));
    assert_eq!(
        source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
        Some(denied())
    );
    assert_eq!(rig.script.count(), 1);
}

#[test]
fn a_declaration_without_an_exceptional_subject_admits_no_exceptional_run() {
    let (mut source, rig) = source();
    let mut declared = support::federated();
    declared.exceptional_subject = String::new();
    declared.exceptional_environment = String::new();
    let repo = repository_with(Visibility::Public, declared);
    let set = pair();
    // A token that names no subject and no environment must not slip through as "equal".
    let mut bare = claims("j1");
    bare["sub"] = json!("");
    bare.as_object_mut().unwrap().remove("environment");
    rig.tokens.push(jwt_of(&bare));
    let req = GrantRequest {
        exceptional: true,
        ..request(&repo, GrantPhase::Stage, &set)
    };
    assert_eq!(source.grant(&req).err(), Some(denied()));
    assert_eq!(rig.tokens.asked(), 0);
    assert_eq!(rig.script.count(), 0);
}

#[test]
fn a_read_and_an_anonymous_identity_are_granted_nothing_and_ask_for_nothing() {
    let (mut source, rig) = source();
    let set = pair();
    let repo = repository();
    let read = source
        .grant(&request(&repo, GrantPhase::Read, &set))
        .unwrap();
    assert!(read.is_none());
    for phase in [GrantPhase::Stage, GrantPhase::Commit, GrantPhase::Read] {
        let mut anonymous = repository();
        anonymous.push = Some(PushIdentity {
            kind: Some(Kind::Anonymous(Anonymous {})),
        });
        let grant = source.grant(&request(&anonymous, phase, &set)).unwrap();
        assert!(grant.is_none(), "{phase:?}");
        anonymous.push = None;
        assert!(
            source
                .grant(&request(&anonymous, phase, &set))
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(rig.tokens.asked(), 0);
    assert_eq!(rig.script.count(), 0);
}

#[test]
fn a_repository_that_is_not_public_or_not_the_one_served_is_refused_before_anything_is_asked() {
    let (mut source, rig) = source();
    let set = pair();
    for visibility in [Visibility::Private, Visibility::Unspecified] {
        let repo = repository_with(visibility, support::federated());
        for phase in [GrantPhase::Stage, GrantPhase::Commit] {
            assert_eq!(
                source.grant(&request(&repo, phase, &set)).err(),
                Some(denied()),
                "{visibility:?} {phase:?}"
            );
        }
    }
    let mut elsewhere = repository();
    elsewhere.location = format!("{LOCATION}/");
    assert_eq!(
        source
            .grant(&request(&elsewhere, GrantPhase::Stage, &set))
            .err(),
        Some(denied())
    );
    assert_eq!(rig.tokens.asked(), 0);
    assert_eq!(rig.script.count(), 0);
}

#[test]
fn a_set_mopm_cannot_hold_is_refused_before_an_identity_token_is_asked_for() {
    let (mut source, rig) = source();
    let repo = repository();
    let a = coordinate("alpha", "1.2.3", "linux-64");
    let cases: Vec<(&str, Vec<Coordinate>, SeamError)> = vec![
        ("empty", vec![], SeamError::Refused(Reason::Channel)),
        (
            "a misspelled name",
            vec![coordinate("Alpha", "1.2.3", "linux-64")],
            SeamError::Refused(Reason::Channel),
        ),
        (
            "a version with a pre-release",
            vec![coordinate("alpha", "1.2.3-rc1", "linux-64")],
            SeamError::Refused(Reason::Channel),
        ),
        (
            "an unknown platform",
            vec![coordinate("alpha", "1.2.3", "win-64")],
            SeamError::Refused(Reason::Channel),
        ),
        (
            "two versions of one name",
            vec![a.clone(), coordinate("alpha", "1.2.4", "linux-64")],
            SeamError::Refused(Reason::Channel),
        ),
        (
            "another channel",
            vec![Coordinate {
                channel: "other".to_owned(),
                ..a.clone()
            }],
            SeamError::Refused(Reason::Channel),
        ),
        (
            "a coordinate twice",
            vec![a.clone(), a.clone()],
            SeamError::Permanent("a coordinate is named twice".to_owned()),
        ),
    ];
    for (what, set, expected) in cases {
        for phase in [GrantPhase::Stage, GrantPhase::Commit] {
            assert_eq!(
                source.grant(&request(&repo, phase, &set)).err(),
                Some(expected.clone()),
                "{what} {phase:?}"
            );
        }
    }
    assert_eq!(rig.tokens.asked(), 0);
    assert_eq!(rig.script.count(), 0);
}

fn many(count: usize) -> Vec<Coordinate> {
    (0..count)
        .map(|n| coordinate(&format!("pkg{n:03}"), "1.0.0", "noarch"))
        .collect()
}

#[test]
fn a_hundred_coordinates_are_one_grant_and_one_more_is_refused_before_the_exchange() {
    let (mut source, rig) = source();
    let repo = repository();
    rig.tokens.push(token("j1"));
    rig.script.push(echo(&iso(600)));
    let at = many(100);
    let grant = source
        .grant(&request(&repo, GrantPhase::Stage, &at))
        .unwrap();
    assert_eq!(decoded(&grant).coordinates.len(), 100);
    let over = many(101);
    assert_eq!(
        source
            .grant(&request(&repo, GrantPhase::Stage, &over))
            .err(),
        Some(SeamError::Refused(Reason::Credential))
    );
    assert_eq!(rig.tokens.asked(), 1);
    assert_eq!(rig.script.count(), 1);
}

#[test]
fn an_identity_token_that_differs_from_the_declaration_is_dropped_unspent() {
    let repo = repository();
    let set = pair();
    let cases: Vec<(&str, Value)> = vec![
        ("issuer", with("iss", json!("https://other.example"))),
        ("audience", with("aud", json!("other.example.com"))),
        (
            "two audiences",
            with("aud", json!([support::AUDIENCE, "x"])),
        ),
        ("subject", with("sub", json!("repo:example-org/other"))),
        ("environment", with("environment", json!("staging"))),
        ("pinned claim", with("ref", json!("refs/heads/dev"))),
        (
            "private source",
            with("repository_visibility", json!("private")),
        ),
        ("expired", with("exp", json!(NOW_S))),
        ("not yet valid", with("nbf", json!(NOW_S + 60))),
        ("issued in the future", with("iat", json!(NOW_S + 60))),
        ("no jti", with("jti", json!(""))),
        ("no repository id", with("repository_id", json!(""))),
    ];
    for (what, c) in cases {
        let (mut source, rig) = source();
        rig.tokens.push(jwt_of(&c));
        assert_eq!(
            source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
            Some(denied()),
            "{what}"
        );
        assert_eq!(rig.script.count(), 0, "{what}");
    }
}

fn with(name: &str, value: Value) -> Value {
    let mut c = claims("j1");
    c[name] = value;
    c
}

#[test]
fn an_identity_token_that_cannot_be_had_or_read_is_refused() {
    let (mut source, rig) = source();
    let repo = repository();
    let set = pair();
    // Nothing queued: the platform gave no token.
    assert_eq!(
        source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
        Some(refused(GrantFault::Unreachable))
    );
    // A token whose payload is not base64url, and one whose payload is not an object.
    rig.tokens.push(b"a.e30Ae.c".to_vec());
    assert_eq!(
        source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
        Some(denied())
    );
    rig.tokens.push(b"h.WzFd.s".to_vec());
    assert_eq!(
        source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
        Some(denied())
    );
    assert_eq!(rig.script.count(), 0);
}

#[test]
fn an_identity_token_is_never_sent_twice_even_when_the_first_answer_was_lost() {
    let (mut source, rig) = source();
    let repo = repository();
    let set = pair();
    // The first exchange is lost: the issuer may have spent the token.
    rig.tokens.push(token("j1"));
    rig.script
        .push(Reply::Raw(Err(TransportError::no_usable_answer("reset"))));
    assert_eq!(
        source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
        Some(refused(GrantFault::Unreachable))
    );
    // The platform hands the same token again: it is refused without a request.
    rig.tokens.push(token("j1"));
    assert_eq!(
        source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
        Some(refused(GrantFault::Used))
    );
    assert_eq!(rig.script.count(), 1);
    // A new token works.
    rig.tokens.push(token("j2"));
    rig.script.push(echo(&iso(600)));
    assert!(
        source
            .grant(&request(&repo, GrantPhase::Stage, &set))
            .is_ok()
    );
    assert_eq!(rig.script.count(), 2);
}

#[test]
fn a_replayed_identity_token_is_refused_across_phases() {
    let (mut source, rig) = source();
    let repo = repository();
    let set = pair();
    rig.tokens.push(token("j1"));
    rig.script.push(echo(&iso(600)));
    assert!(
        source
            .grant(&request(&repo, GrantPhase::Stage, &set))
            .is_ok()
    );
    rig.tokens.push(token("j1"));
    assert_eq!(
        source
            .grant(&request(&repo, GrantPhase::Commit, &set))
            .err(),
        Some(refused(GrantFault::Used))
    );
    assert_eq!(rig.script.count(), 1);
}

fn one_failure(reply: Reply) -> (SeamError, usize) {
    let (mut source, rig) = source();
    let repo = repository();
    let set = pair();
    rig.tokens.push(token("j1"));
    rig.script.push(reply);
    let error = source
        .grant(&request(&repo, GrantPhase::Stage, &set))
        .err()
        .unwrap();
    (error, rig.script.count())
}

fn mopm_error(code: &str, message: &str) -> String {
    json!({"error": {"code": code, "message": message, "hint": "h"}}).to_string()
}

#[test]
fn an_exchange_that_mopm_refuses_is_classified_and_sent_once() {
    let used = mopm_error(
        "oidc_token_invalid",
        "The CI identity token was rejected: token already used.",
    );
    let other_rejection = mopm_error(
        "oidc_token_invalid",
        "The CI identity token was rejected: wrong audience.",
    );
    let cases: Vec<(&str, Reply, SeamError)> = vec![
        ("used", raw(401, &[], &used), refused(GrantFault::Used)),
        (
            "rejected for another reason",
            raw(401, &[], &other_rejection),
            denied(),
        ),
        (
            "used message under another code",
            raw(401, &[], &mopm_error("token_unknown", "token already used")),
            denied(),
        ),
        ("unparsable 401", raw(401, &[], "<html>"), denied()),
        (
            "no message",
            raw(
                401,
                &[],
                &json!({"error": {"code": "oidc_token_invalid"}}).to_string(),
            ),
            denied(),
        ),
        (
            "not trusted",
            raw(403, &[], &mopm_error("oidc_not_trusted", "m")),
            denied(),
        ),
        (
            "not an owner",
            raw(403, &[], &mopm_error("not_owner", "m")),
            denied(),
        ),
        (
            "rate limited",
            raw(
                429,
                &[("retry-after", "7")],
                &mopm_error("rate_limited", "m"),
            ),
            SeamError::RateLimited(Backoff { ticks: 7 }),
        ),
        (
            "rate limited for a day",
            raw(429, &[("retry-after", "86400")], ""),
            SeamError::Refused(Reason::Channel),
        ),
        (
            "no such route",
            raw(404, &[], &mopm_error("route_not_found", "m")),
            SeamError::Refused(Reason::Channel),
        ),
        (
            "another 404",
            raw(404, &[], &mopm_error("package_not_found", "m")),
            SeamError::Permanent("the exchange answered 404".to_owned()),
        ),
        (
            "invalid name",
            raw(400, &[], &mopm_error("invalid_name", "m")),
            SeamError::Permanent("the exchange answered 400".to_owned()),
        ),
        ("500", raw(500, &[], ""), refused(GrantFault::Unreachable)),
        (
            "502",
            raw(502, &[], "<html>"),
            refused(GrantFault::Unreachable),
        ),
        (
            "503",
            raw(503, &[], &mopm_error("unavailable", "m")),
            refused(GrantFault::Unreachable),
        ),
        (
            "never connected",
            Reply::Raw(Err(TransportError::never_connected("refused"))),
            refused(GrantFault::Unreachable),
        ),
        (
            "no usable answer",
            Reply::Raw(Err(TransportError::no_usable_answer("reset"))),
            refused(GrantFault::Unreachable),
        ),
    ];
    for (what, reply, expected) in cases {
        let (error, sent) = one_failure(reply);
        assert_eq!(error, expected, "{what}");
        assert_eq!(sent, 1, "{what}");
    }
}

fn ok_body(mutate: impl FnOnce(&mut Value)) -> Reply {
    let mut body = json!({
        "token": MINTED,
        "token_kind": "publish",
        "expires_at": iso(600),
        "packages": [
            {"name": "alpha", "version": "1.2.3"},
            {"name": "beta", "version": "0.1.0"},
        ],
    });
    mutate(&mut body);
    raw(200, &[], &body.to_string())
}

#[test]
fn an_answer_that_is_not_a_well_formed_grant_is_unreachable() {
    let bad_times = [
        "",
        "soon",
        "2027-01-15",
        "2027-01-15T08:10:00",
        "2027-01-15 08:10:00.000Z",
        "2027-01-15T08:10:00.Z",
        "2027-01-15T08:10:00.1234567890Z",
        "2027-01-15T08:10:00.5xZ",
        "2027-01-15T08:10:00+00:00",
        "2027-13-15T08:10:00Z",
        "2027-00-15T08:10:00Z",
        "2027-01-32T08:10:00Z",
        "2027-01-00T08:10:00Z",
        "2027-02-29T08:10:00Z",
        "2100-02-29T08:10:00Z",
        "2027-04-31T08:10:00Z",
        "2027-01-15T24:10:00Z",
        "2027-01-15T08:60:00Z",
        "2027-01-15T08:10:60Z",
        "0000-12-31T23:59:59Z",
        "2027-01-15T08:10:00\u{e9}Z",
        "2027/01/15T08:10:00Z",
        "20x7-01-15T08:10:00Z",
        "2027-01-15T08:10:00ZZ",
    ];
    let mut replies: Vec<(String, Reply)> = vec![
        ("not json".to_owned(), raw(200, &[], "<html>")),
        ("an array".to_owned(), raw(200, &[], "[]")),
        (
            "no token".to_owned(),
            ok_body(|b| {
                b.as_object_mut().unwrap().remove("token");
            }),
        ),
        (
            "token not text".to_owned(),
            ok_body(|b| b["token"] = json!(5)),
        ),
        (
            "empty token".to_owned(),
            ok_body(|b| b["token"] = json!("")),
        ),
        (
            "token with a space".to_owned(),
            ok_body(|b| b["token"] = json!("a b")),
        ),
        (
            "no expiry".to_owned(),
            ok_body(|b| {
                b.as_object_mut().unwrap().remove("expires_at");
            }),
        ),
        (
            "expiry not text".to_owned(),
            ok_body(|b| b["expires_at"] = json!(5)),
        ),
        (
            "no packages".to_owned(),
            ok_body(|b| {
                b.as_object_mut().unwrap().remove("packages");
            }),
        ),
        (
            "packages not a list".to_owned(),
            ok_body(|b| b["packages"] = json!("x")),
        ),
        (
            "a package without a name".to_owned(),
            ok_body(|b| b["packages"] = json!([{"version": "1"}])),
        ),
        (
            "a package without a version".to_owned(),
            ok_body(|b| b["packages"] = json!([{"name": "alpha"}])),
        ),
        (
            "a package name not text".to_owned(),
            ok_body(|b| b["packages"] = json!([{"name": 1, "version": "1"}])),
        ),
        (
            "a package version not text".to_owned(),
            ok_body(|b| b["packages"] = json!([{"name": "alpha", "version": 1}])),
        ),
    ];
    for time in bad_times {
        let time = time.to_owned();
        replies.push((
            format!("expiry {time:?}"),
            ok_body(|b| b["expires_at"] = json!(time)),
        ));
    }
    for (what, reply) in replies {
        let (error, sent) = one_failure(reply);
        assert_eq!(error, refused(GrantFault::Unreachable), "{what}");
        assert_eq!(sent, 1);
    }
}

#[test]
fn an_answer_that_contradicts_the_request_is_denied() {
    let replies: Vec<(&str, Reply)> = vec![
        (
            "a stage token",
            ok_body(|b| b["token_kind"] = json!("stage")),
        ),
        (
            "no kind",
            ok_body(|b| {
                b.as_object_mut().unwrap().remove("token_kind");
            }),
        ),
        ("a kind not text", ok_body(|b| b["token_kind"] = json!(1))),
        (
            "an extra package",
            ok_body(|b| {
                b["packages"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"name": "gamma", "version": "1.0.0"}))
            }),
        ),
        (
            "a missing package",
            ok_body(|b| {
                b["packages"].as_array_mut().unwrap().pop();
            }),
        ),
        (
            "another version",
            ok_body(|b| b["packages"][1]["version"] = json!("0.1.1")),
        ),
        ("no packages at all", ok_body(|b| b["packages"] = json!([]))),
    ];
    for (what, reply) in replies {
        let (error, sent) = one_failure(reply);
        assert_eq!(error, denied(), "{what}");
        assert_eq!(sent, 1);
    }
}

fn expiry_of(expires_at: &str, now_ms: u64) -> Result<Option<u64>, SeamError> {
    let (mut source, rig) = source_for(ActorForm::ConfiguredCi, &ManualClock::at(now_ms));
    let repo = repository();
    let set = pair();
    let mut claims = claims("j1");
    claims["exp"] = json!(now_ms / 1000 + 300);
    claims["iat"] = json!(now_ms / 1000);
    claims["nbf"] = json!(now_ms / 1000);
    rig.tokens.push(jwt_of(&claims));
    rig.script.push(echo(expires_at));
    source
        .grant(&request(&repo, GrantPhase::Commit, &set))
        .map(|g| g.expires_ms())
}

#[test]
fn the_stated_expiry_is_read_to_the_millisecond_on_known_dates() {
    // 2038-01-19T03:14:07Z is 2147483647 seconds.
    let now = 2_147_483_100_000;
    assert_eq!(
        expiry_of("2038-01-19T03:14:07Z", now),
        Ok(Some(2_147_483_647_000))
    );
    assert_eq!(
        expiry_of("2038-01-19T03:14:07.5Z", now),
        Ok(Some(2_147_483_647_500))
    );
    assert_eq!(
        expiry_of("2038-01-19T03:14:07.123456789Z", now),
        Ok(Some(2_147_483_647_123))
    );
    assert_eq!(
        expiry_of("2038-01-19T03:14:07.04Z", now),
        Ok(Some(2_147_483_647_040))
    );
    // A century year that is not a leap year.
    let now = 4_107_542_400_000 - 300_000;
    assert_eq!(
        expiry_of("2100-03-01T00:00:00.000Z", now),
        Ok(Some(4_107_542_400_000))
    );
    // A leap day.
    let day = 21_243 * 86_400_000;
    assert_eq!(
        expiry_of("2028-02-29T00:10:00.000Z", day),
        Ok(Some(day + 600_000))
    );
    let day = 21_244 * 86_400_000 - 300_000;
    assert_eq!(
        expiry_of("2028-03-01T00:00:00Z", day),
        Ok(Some(day + 300_000))
    );
    // 2400 is a leap year.
    let day = 13_574_563_200_000;
    assert_eq!(
        expiry_of("2400-02-29T00:00:00Z", day - 300_000),
        Ok(Some(day))
    );
    assert_eq!(expiry_of(&iso(600), NOW_MS), Ok(Some(NOW_MS + 600_000)));
}

#[test]
fn a_token_never_outlives_ten_minutes_on_our_clock_whatever_the_issuer_says() {
    // An issuer whose clock is an hour ahead states an expiry an hour and ten minutes away.
    assert_eq!(expiry_of(&iso(4200), NOW_MS), Ok(Some(NOW_MS + 600_000)));
    // A shorter life is kept as stated.
    assert_eq!(expiry_of(&iso(60), NOW_MS), Ok(Some(NOW_MS + 60_000)));
    assert_eq!(expiry_of(&iso(1), NOW_MS), Ok(Some(NOW_MS + 1000)));
}

#[test]
fn an_expiry_that_is_not_after_now_is_expired() {
    assert_eq!(
        expiry_of(&iso(0), NOW_MS),
        Err(refused(GrantFault::Expired))
    );
    assert_eq!(
        expiry_of(&iso(0), NOW_MS + 1),
        Err(refused(GrantFault::Expired))
    );
    assert_eq!(
        expiry_of("2027-01-15T07:00:00.000Z", NOW_MS),
        Err(refused(GrantFault::Expired))
    );
}

#[test]
fn the_grant_issued_is_good_for_a_call_that_ends_inside_the_token_and_not_after() {
    let (mut source, rig) = source();
    let repo = repository();
    let set = pair();
    rig.tokens.push(token("j1"));
    rig.script.push(echo(&iso(600)));
    let grant = source
        .grant(&request(&repo, GrantPhase::Stage, &set))
        .unwrap();
    let refs: Vec<&Coordinate> = set.iter().collect();
    let at = |now_ms: u64, lasts_ms: u64| {
        authorise(
            &grant,
            &Use {
                registry: LOCATION,
                now_ms,
                verb: Verb::Stage,
                coordinates: &refs,
                lasts_ms,
            },
        )
        .err()
    };
    assert_eq!(at(NOW_MS, 359_999), None);
    assert_eq!(at(NOW_MS + 599_999, 0), None);
    assert_eq!(at(NOW_MS + 600_000, 0), Some(refused(GrantFault::Expired)));
    assert_eq!(at(NOW_MS, 600_000), Some(refused(GrantFault::Expired)));
    // A stage grant does not commit.
    let commit = Use {
        registry: LOCATION,
        now_ms: NOW_MS,
        verb: Verb::Commit,
        coordinates: &refs,
        lasts_ms: 0,
    };
    assert_eq!(authorise(&grant, &commit).err(), Some(denied()));
}

#[test]
fn neither_token_reaches_an_error_a_log_or_the_grant_beyond_its_one_place() {
    let jwt = String::from_utf8(token("j1")).unwrap();
    let signature = "CANARY-signature-abc";
    let leak =
        |text: &str| text.contains(&jwt) || text.contains(signature) || text.contains(MINTED);
    let repo = repository();
    let set = pair();

    // Every way an exchange can end, each with a server message that echoes the secrets.
    let echoing = |status: u16| {
        let body = mopm_error(
            "oidc_token_invalid",
            &format!("echo {jwt} {MINTED} token already used"),
        );
        raw(status, &[], &body)
    };
    let replies: Vec<Reply> = vec![
        echoing(401),
        echoing(403),
        echoing(404),
        echoing(429),
        echoing(500),
        raw(200, &[], &format!("{jwt} {MINTED}")),
        Reply::Raw(Err(TransportError::no_usable_answer("reset"))),
        ok_body(|b| b["token_kind"] = json!("stage")),
        echo(&iso(0)),
    ];
    for reply in replies {
        let (mut source, rig) = source();
        rig.tokens.push(token("j1"));
        rig.script.push(reply);
        let error = source
            .grant(&request(&repo, GrantPhase::Stage, &set))
            .err()
            .unwrap();
        assert!(!leak(&texts(&error)), "{error}");
        for line in rig.script.debugs() {
            assert!(!leak(&line), "{line}");
        }
    }

    // The grant that is issued holds the minted token once, and not the identity token.
    let (mut source, rig) = source();
    rig.tokens.push(token("j1"));
    rig.script.push(echo(&iso(600)));
    let grant = source
        .grant(&request(&repo, GrantPhase::Stage, &set))
        .unwrap();
    let secret = String::from_utf8_lossy(grant.secret()).into_owned();
    assert_eq!(secret.matches(MINTED).count(), 1);
    assert!(!secret.contains(signature));
    assert!(!secret.contains(&jwt));
    for line in rig.script.debugs() {
        assert!(!leak(&line), "{line}");
    }
    // The only request that carries the identity token is the exchange, in its body; it has no
    // authorization header.
    let sent = rig.script.sent.lock().unwrap();
    assert!(String::from_utf8_lossy(&sent[0].body).contains(&jwt));
    assert!(!sent[0].headers.iter().any(|h| h == "authorization"));
}

#[test]
fn a_refusal_before_the_exchange_names_no_token_either() {
    let jwt = String::from_utf8(token("j1")).unwrap();
    let repo = repository();
    let set = pair();
    let (mut source, rig) = source();
    let mut wrong = claims("j1");
    wrong["sub"] = json!("repo:someone/else");
    let wrong_jwt = String::from_utf8(jwt_of(&wrong)).unwrap();
    rig.tokens.push(wrong_jwt.clone().into_bytes());
    let error = source
        .grant(&request(&repo, GrantPhase::Stage, &set))
        .err()
        .unwrap();
    assert!(!texts(&error).contains(&wrong_jwt));
    assert!(!texts(&error).contains("someone"));
}
