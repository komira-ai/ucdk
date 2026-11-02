// SPDX-License-Identifier: Apache-2.0
//! The mopm grant source: what it checks before it asks for an identity token, what it sends,
//! and that an identity token is never sent twice. How the answer is read is in
//! `exchange_answer.rs`.

mod common;
mod support;

use std::collections::BTreeMap;

use common::coordinate;
use serde_json::{Value, json};
use support::{
    LOCATION, MINTED, NOW_MS, NOW_S, Reply, claims, decoded, denied, echo, iso, jwt_of, mopm_error,
    pair, raw, refused, repository, repository_with, request, source, source_for, source_with,
    token,
};
use ucdk_ledger::ManualClock;
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_mopm::{ActorForm, Method, TransportError, Url};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{Anonymous, Coordinate, PushIdentity, Reason, Visibility};
use ucdk_release_seams::{GrantRequest, GrantSource, SeamError};

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
    for location in [
        format!("{LOCATION}/"),
        "https://registry.example.com/api/v2".to_owned(),
        "https://other.example.com/api/v1".to_owned(),
        "https://registry.example.com:8443/api/v1".to_owned(),
        String::new(),
    ] {
        let mut elsewhere = repository();
        elsewhere.location = location.clone();
        assert_eq!(
            source
                .grant(&request(&elsewhere, GrantPhase::Stage, &set))
                .err(),
            Some(denied()),
            "{location}"
        );
    }
    assert_eq!(rig.tokens.asked(), 0);
    assert_eq!(rig.script.count(), 0);
}

#[test]
fn a_source_serves_only_the_registry_it_exchanges_at() {
    let set = pair();
    // The exchange registry differs from the repository's location by path, by origin, and by
    // a query (a configuration the check refuses): the repository is refused unasked.
    for registry in [
        "https://registry.example.com/api/v2",
        "https://other.example.com/api/v1",
        "https://registry.example.com/api/v1?k=v",
    ] {
        let mut config = common::config();
        config.registry = Url::parse(registry).unwrap();
        let (mut source, rig) =
            source_with(config, ActorForm::ConfiguredCi, &ManualClock::at(NOW_MS));
        rig.tokens.push(token("j1"));
        rig.script.push(echo(&iso(600)));
        let repo = repository();
        assert_eq!(
            source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
            Some(denied()),
            "{registry}"
        );
        assert_eq!(rig.tokens.asked(), 0, "{registry}");
        assert_eq!(rig.script.count(), 0, "{registry}");
    }
    // When they agree, the envelope names the registry the exchange was sent to.
    let mut config = common::config();
    config.registry = Url::parse("https://other.example.com/v9").unwrap();
    let (mut source, rig) = source_with(config, ActorForm::ConfiguredCi, &ManualClock::at(NOW_MS));
    rig.tokens.push(token("j1"));
    rig.script.push(echo(&iso(600)));
    let mut repo = repository();
    repo.location = "https://other.example.com/v9".to_owned();
    let grant = source
        .grant(&request(&repo, GrantPhase::Stage, &set))
        .unwrap();
    assert_eq!(decoded(&grant).registry, "https://other.example.com/v9");
    assert_eq!(
        rig.script.sent.lock().unwrap()[0].url,
        "https://other.example.com/v9/auth/oidc/exchange"
    );
}

#[test]
fn a_set_mopm_cannot_hold_is_refused_before_an_identity_token_is_asked_for() {
    let (mut source, rig) = source();
    let repo = repository();
    let a = coordinate("alpha", "1.2.3", "linux-x86_64");
    let cases: Vec<(&str, Vec<Coordinate>, SeamError)> = vec![
        ("empty", vec![], SeamError::Refused(Reason::Channel)),
        (
            "a misspelled name",
            vec![coordinate("Alpha", "1.2.3", "linux-x86_64")],
            SeamError::Refused(Reason::Channel),
        ),
        (
            "a version with a pre-release",
            vec![coordinate("alpha", "1.2.3-rc1", "linux-x86_64")],
            SeamError::Refused(Reason::Channel),
        ),
        (
            "an unknown platform",
            vec![coordinate("alpha", "1.2.3", "win-64")],
            SeamError::Refused(Reason::Channel),
        ),
        (
            "two versions of one name",
            vec![a.clone(), coordinate("alpha", "1.2.4", "linux-x86_64")],
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
        .map(|n| coordinate(&format!("pkg{n:03}"), "1.0.0", "any"))
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
        ("issuer", with("iss", json!("https://other.example.com"))),
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

#[test]
fn a_declaration_of_several_pinned_claims_holds_each_one_before_the_exchange() {
    let set = pair();
    let pins = BTreeMap::from([
        ("environment_node_id".to_owned(), "EN_1".to_owned()),
        ("ref".to_owned(), "refs/heads/main".to_owned()),
        ("repository_owner".to_owned(), "example-org".to_owned()),
    ]);
    let mut declared = support::federated();
    declared.pinned_claims = pins.clone();
    let repo = repository_with(Visibility::Public, declared);
    let mut good = claims("j1");
    for (name, value) in &pins {
        good[name] = json!(value);
    }
    for name in pins.keys() {
        for change in [Some(json!("other")), None] {
            let mut c = good.clone();
            match &change {
                Some(value) => c[name] = value.clone(),
                None => {
                    c.as_object_mut().unwrap().remove(name);
                }
            }
            let (mut source, rig) = source();
            rig.tokens.push(jwt_of(&c));
            assert_eq!(
                source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
                Some(denied()),
                "{name} {change:?}"
            );
            assert_eq!(rig.script.count(), 0, "{name} {change:?}");
        }
    }
    let (mut source, rig) = source();
    rig.tokens.push(jwt_of(&good));
    rig.script.push(echo(&iso(600)));
    assert!(
        source
            .grant(&request(&repo, GrantPhase::Stage, &set))
            .is_ok()
    );
    assert_eq!(rig.script.count(), 1);
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
    // A token that is not three base64url parts: the platform gave no usable token.
    rig.tokens.push(b"a.e30Ae.c".to_vec());
    assert_eq!(
        source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
        Some(refused(GrantFault::Unreachable))
    );
    // A token whose payload is not an object, and one whose payload names a claim twice.
    let mut twice = claims("j1").to_string();
    twice.insert_str(twice.len() - 1, r#","iss":"https://other.example.com""#);
    rig.tokens
        .push(format!("e30.{}.c2ln", support::b64(twice.as_bytes())).into_bytes());
    assert_eq!(
        source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
        Some(denied())
    );
    rig.tokens.push(b"e30.WzFd.c2ln".to_vec());
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

#[test]
fn a_token_id_once_sent_stays_spent_whatever_the_bytes_and_whatever_the_answer() {
    let repo = repository();
    let set = pair();
    // Each way an exchange can end once the request went out.
    let endings: Vec<(&str, Reply)> = vec![
        ("granted", echo(&iso(600))),
        (
            "refused",
            raw(401, &[], &mopm_error("oidc_not_trusted", "m")),
        ),
        ("forbidden", raw(403, &[], &mopm_error("not_owner", "m"))),
        ("unreadable", raw(200, &[], "<html>")),
        ("expired", echo(&iso(0))),
        (
            "lost",
            Reply::Raw(Err(TransportError::no_usable_answer("reset"))),
        ),
    ];
    for (what, ending) in endings {
        let (mut source, rig) = source();
        rig.tokens.push(token("j1"));
        rig.script.push(ending);
        let _ = source.grant(&request(&repo, GrantPhase::Stage, &set));
        assert_eq!(rig.script.count(), 1, "{what}");
        // Other bytes with the same `jti` (a later `iat`) are the same token to the issuer.
        let mut later = claims("j1");
        later["iat"] = json!(NOW_S);
        rig.tokens.push(jwt_of(&later));
        assert_eq!(
            source
                .grant(&request(&repo, GrantPhase::Commit, &set))
                .err(),
            Some(refused(GrantFault::Used)),
            "{what}"
        );
        assert_eq!(rig.script.count(), 1, "{what}");
    }
}

#[test]
fn a_token_refused_before_it_was_sent_does_not_spend_its_id() {
    let repo = repository();
    let set = pair();
    let (mut source, rig) = source();
    // Refused for its claims: never sent.
    let mut wrong = claims("j1");
    wrong["sub"] = json!("repo:someone/else");
    rig.tokens.push(jwt_of(&wrong));
    assert_eq!(
        source.grant(&request(&repo, GrantPhase::Stage, &set)).err(),
        Some(denied())
    );
    // Refused for the set, before a token was asked for.
    assert_eq!(
        source.grant(&request(&repo, GrantPhase::Stage, &[])).err(),
        Some(SeamError::Refused(Reason::Channel))
    );
    assert_eq!(rig.script.count(), 0);
    // A good token with that id is then exchanged.
    rig.tokens.push(token("j1"));
    rig.script.push(echo(&iso(600)));
    assert!(
        source
            .grant(&request(&repo, GrantPhase::Stage, &set))
            .is_ok()
    );
    assert_eq!(rig.script.count(), 1);
}
