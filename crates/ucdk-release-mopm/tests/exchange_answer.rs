// SPDX-License-Identifier: Apache-2.0
//! The answer to an exchange: how a refusal is classified, how a granted answer is read (its
//! token, its expiry, its packages), and that no credential reaches an error or a log.

mod common;
mod support;

use std::time::Duration;

use serde_json::{Value, json};
use support::{
    LOCATION, MINTED, NOW_MS, Reply, claims, decoded, denied, echo, iso, jwt_of, mopm_error, pair,
    raw, refused, repository, request, source, source_for, texts, token,
};
use ucdk_ledger::{Clock, ManualClock};
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_mopm::{
    ActorForm, DEFAULT_RETRY_AFTER_SECONDS, TransportError, Use, Verb, authorise,
};
use ucdk_release_proto::v1::{Coordinate, Reason};
use ucdk_release_seams::{Backoff, GrantSource, SeamError};

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
    raw(200, &[], &ok_text(mutate))
}

/// A good answer whose first package is written as `first`, raw, and whose second is beta's.
fn packages(first: &str) -> Reply {
    let text = format!(
        r#"{{"token":"{MINTED}","token_kind":"publish","expires_at":"{}","packages":[{first},{{"name":"beta","version":"0.1.0"}}]}}"#,
        iso(600)
    );
    raw(200, &[], &text)
}

/// A good answer with `extra` written as further fields after the others.
fn repeated(extra: &str) -> Reply {
    let mut text = ok_text(|_| {});
    text.insert_str(text.len() - 1, &format!(",{extra}"));
    raw(200, &[], &text)
}

fn ok_text(mutate: impl FnOnce(&mut Value)) -> String {
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
    body.to_string()
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
        "2027-01-15T08:10:00\u{e9}Z",
        "2027/01/15T08:10:00Z",
        "abcd-01-15T08:10:00Z",
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
            ok_body(|b| {
                b["token"] = json!(5);
            }),
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
        (
            "the token twice".to_owned(),
            repeated(r#""token":"other-token""#),
        ),
        (
            "the expiry twice".to_owned(),
            repeated(&format!(r#""expires_at":"{}""#, iso(60))),
        ),
        (
            "the kind twice".to_owned(),
            repeated(r#""token_kind":"stage""#),
        ),
        (
            "an unknown field twice".to_owned(),
            repeated(r#""note":1,"note":2"#),
        ),
        (
            "a package name twice, the asked one last".to_owned(),
            packages(r#"{"name":"other","name":"alpha","version":"1.2.3"}"#),
        ),
        (
            "a package name twice, the asked one first".to_owned(),
            packages(r#"{"name":"alpha","name":"other","version":"1.2.3"}"#),
        ),
        (
            "a package version twice, the asked one last".to_owned(),
            packages(r#"{"name":"alpha","version":"9.9.9","version":"1.2.3"}"#),
        ),
        (
            "a package version twice, the asked one first".to_owned(),
            packages(r#"{"name":"alpha","version":"1.2.3","version":"9.9.9"}"#),
        ),
        (
            "an object and more".to_owned(),
            raw(200, &[], &format!("{} x", ok_text(|_| {}))),
        ),
    ];
    // Instants before the epoch, built here so that no early date is written in the source.
    let before_epoch = [
        format!("{:04}-12-31T23:59:59Z", 0),
        format!("{:04}-01-01T00:00:00Z", 0),
        format!("{:04}-02-28T00:00:00Z", 0),
    ];
    for time in bad_times
        .iter()
        .map(ToString::to_string)
        .chain(before_epoch)
    {
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
                    .push(json!({"name": "gamma", "version": "1.0.0"}));
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
        (
            "a package repeated",
            ok_body(|b| {
                b["packages"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"name": "beta", "version": "0.1.0"}));
            }),
        ),
    ];
    for (what, reply) in replies {
        let (error, sent) = one_failure(reply);
        assert_eq!(error, denied(), "{what}");
        assert_eq!(sent, 1);
    }
}

#[test]
fn an_answer_with_more_fields_or_its_packages_in_another_order_is_read() {
    let replies: Vec<(&str, Reply)> = vec![
        (
            "fields mopm may add",
            ok_body(|b| {
                b["scope"] = json!(["publish"]);
                b["issued_at"] = json!(iso(0));
                b["packages"][0]["created"] = json!(true);
                b["packages"][1]["note"] = json!({"name": "x", "version": "y"});
            }),
        ),
        (
            "packages reversed",
            ok_body(|b| b["packages"].as_array_mut().unwrap().reverse()),
        ),
    ];
    for (what, reply) in replies {
        let (mut source, rig) = source();
        rig.tokens.push(token("j1"));
        rig.script.push(reply);
        let repo = repository();
        let set = pair();
        let grant = source
            .grant(&request(&repo, GrantPhase::Stage, &set))
            .unwrap();
        assert_eq!(
            decoded(&grant).bearer_token(),
            Some(MINTED.as_bytes()),
            "{what}"
        );
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

/// A grant issued over a network that takes `delay` to deliver the answer, from an issuer whose
/// clock is `ahead_s` seconds ahead and states a ten-minute life.
fn over_a_slow_network(delay: Duration, ahead_s: u64) -> (Result<Option<u64>, SeamError>, u64) {
    let clock = ManualClock::at(NOW_MS);
    let (mut source, rig) = source_for(ActorForm::ConfiguredCi, &clock);
    rig.script.delay(&clock, delay);
    rig.tokens.push(token("j1"));
    rig.script.push(echo(&iso(ahead_s + 600)));
    let repo = repository();
    let set = pair();
    let result = source
        .grant(&request(&repo, GrantPhase::Stage, &set))
        .map(|g| g.expires_ms());
    (result, clock.now_millis())
}

#[test]
fn a_slow_answer_does_not_add_its_delivery_time_to_the_life_of_the_token() {
    // The life is counted from when the request was sent: an answer two minutes late from an
    // issuer an hour ahead leaves eight minutes, not ten.
    let (expiry, received) = over_a_slow_network(Duration::from_secs(120), 3600);
    assert_eq!(received, NOW_MS + 120_000);
    assert_eq!(expiry, Ok(Some(NOW_MS + 600_000)));
    // An answer that arrives when the ten minutes are up, or after, is expired on arrival.
    for late in [600, 601, 3600] {
        let (expiry, _) = over_a_slow_network(Duration::from_secs(late), 3600);
        assert_eq!(expiry, Err(refused(GrantFault::Expired)), "{late}");
    }
    // One millisecond inside the ten minutes is a grant of one millisecond.
    let (expiry, _) = over_a_slow_network(Duration::from_millis(599_999), 3600);
    assert_eq!(expiry, Ok(Some(NOW_MS + 600_000)));
    // An issuer on time states its own expiry, which the delay does not move either.
    let (expiry, _) = over_a_slow_network(Duration::from_secs(120), 0);
    assert_eq!(expiry, Ok(Some(NOW_MS + 600_000)));
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
    let expected = [
        refused(GrantFault::Used),
        denied(),
        SeamError::Permanent("the exchange answered 404".to_owned()),
        SeamError::RateLimited(Backoff {
            ticks: u32::try_from(DEFAULT_RETRY_AFTER_SECONDS).unwrap(),
        }),
        refused(GrantFault::Unreachable),
        refused(GrantFault::Unreachable),
        refused(GrantFault::Unreachable),
        denied(),
        refused(GrantFault::Expired),
    ];
    for (reply, expected) in replies.into_iter().zip(expected) {
        let (mut source, rig) = source();
        rig.tokens.push(token("j1"));
        rig.script.push(reply);
        let error = source
            .grant(&request(&repo, GrantPhase::Stage, &set))
            .err()
            .unwrap();
        // The whole diagnostic is the one of the expected error, which is built from no secret.
        assert_eq!(texts(&error), texts(&expected));
        assert!(!leak(&texts(&error)), "{error}");
        assert_eq!(rig.script.debugs(), [exchange_log(&rig)]);
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
    assert_eq!(rig.script.debugs(), [exchange_log(&rig)]);
    for line in rig.script.debugs() {
        assert!(!leak(&line), "{line}");
    }
    // The only request that carries the identity token is the exchange, in its body; it has no
    // authorization header.
    let sent = rig.script.sent.lock().unwrap();
    assert!(String::from_utf8_lossy(&sent[0].body).contains(&jwt));
    assert!(!sent[0].headers.iter().any(|h| h == "authorization"));
}

/// The whole line a log holds for the one exchange `rig` was sent: no URL query, no header
/// value but the plain ones, and only the length of the body.
fn exchange_log(rig: &support::Rig) -> String {
    let body_len = rig.script.sent.lock().unwrap()[0].body.len();
    format!(
        concat!(
            r#"Request {{ method: "POST", "#,
            r#"url: "https://registry.example.com/api/v1/auth/oidc/exchange", "#,
            r#"headers: ["content-type: application/json", "accept: application/json"], "#,
            r#"body_len: {} }}"#
        ),
        body_len
    )
}

#[test]
fn a_refusal_before_the_exchange_names_no_token_either() {
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
    assert_eq!(texts(&error), texts(&denied()));
    assert!(!texts(&error).contains(&wrong_jwt));
    assert!(!texts(&error).contains("someone"));
}
