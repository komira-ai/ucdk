// SPDX-License-Identifier: Apache-2.0
//! The grant envelope and the client-side checks of a grant: every refusal sends nothing, and
//! the answers of mopm that mean a bad grant are classified.

mod common;

use std::time::Duration;

use common::{coordinate, error_body, response};
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_mopm::{Envelope, Mapped, Op, Use, Verb, authorise, map_reply};
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::{Grant, SeamError};

const REGISTRY: &str = "https://registry.example.com/api/v1";
const TOKEN: &[u8] = b"CANARY-token-0123456789";
const NOW: u64 = 1_000_000;
const WRITE_VERBS: [Verb; 4] = [Verb::Stage, Verb::Abort, Verb::Commit, Verb::Yank];
const READ_VERBS: [Verb; 4] = [Verb::Observe, Verb::Fetch, Verb::Newest, Verb::List];

fn set() -> Vec<Coordinate> {
    vec![
        coordinate("alpha", "1.2.3", "linux-64"),
        coordinate("alpha", "1.2.3", "osx-arm64"),
        coordinate("beta", "0.1.0", "noarch"),
    ]
}

fn envelope(phase: GrantPhase, coordinates: Vec<Coordinate>) -> Envelope {
    let token = if phase == GrantPhase::Read {
        Vec::new()
    } else {
        TOKEN.to_vec()
    };
    Envelope::new(
        phase,
        false,
        REGISTRY.to_owned(),
        "ci:gh-repo:9001".to_owned(),
        coordinates,
        token,
    )
}

fn grant(phase: GrantPhase, coordinates: Vec<Coordinate>, expires_ms: u64) -> Grant {
    Grant::issued(envelope(phase, coordinates).encode(), expires_ms)
}

fn call<'a>(verb: Verb, coordinates: &'a [&'a Coordinate]) -> Use<'a> {
    Use {
        registry: REGISTRY,
        now_ms: NOW,
        verb,
        coordinates,
        lasts_ms: 0,
    }
}

fn denied() -> SeamError {
    SeamError::GrantRefused(GrantFault::Denied)
}

fn expired() -> SeamError {
    SeamError::GrantRefused(GrantFault::Expired)
}

fn frame(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend_from_slice(part.len().to_string().as_bytes());
        out.push(b':');
        out.extend_from_slice(part);
    }
    out
}

#[test]
fn an_envelope_round_trips_every_field_even_when_a_text_holds_a_separator() {
    let tricky = Coordinate {
        channel: "a:b".to_owned(),
        r#type: 7,
        name: "1:x".to_owned(),
        version: String::new(),
        platform: "5:abcde".to_owned(),
    };
    let original = Envelope::new(
        GrantPhase::Commit,
        true,
        "https://r.example/x:y".to_owned(),
        "u:42".to_owned(),
        vec![tricky.clone(), coordinate("alpha", "1.2.3", "noarch")],
        TOKEN.to_vec(),
    );
    let back = Envelope::decode(&original.encode()).unwrap();
    assert_eq!(back.phase, GrantPhase::Commit);
    assert!(back.exceptional);
    assert_eq!(back.registry, "https://r.example/x:y");
    assert_eq!(back.actor, "u:42");
    assert_eq!(back.coordinates[0], tricky);
    assert_eq!(back.coordinates.len(), 2);
    assert_eq!(back.bearer_token(), Some(TOKEN));
    for phase in [GrantPhase::Stage, GrantPhase::Read] {
        let other = envelope(phase, Vec::new());
        let decoded = Envelope::decode(&other.encode()).unwrap();
        assert_eq!(decoded.phase, phase);
        assert!(!decoded.exceptional);
    }
}

#[test]
fn a_read_envelope_has_no_bearer() {
    let read = envelope(GrantPhase::Read, set());
    assert_eq!(read.bearer_token(), None);
    assert_eq!(
        Envelope::decode(&read.encode()).unwrap().bearer_token(),
        None
    );
}

#[test]
fn bytes_that_are_not_an_envelope_are_malformed() {
    let good = envelope(GrantPhase::Stage, set()).encode();
    let token_frame = |token: &[u8]| frame(&[b"1", b"stage", b"0", b"r", b"a", b"0", token]);
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("no colon", b"11".to_vec()),
        ("length not utf-8", b"\xff:x".to_vec()),
        ("length not a number", b"x:x".to_vec()),
        ("signed length", b"+1:1".to_vec()),
        ("leading zero", b"01:1".to_vec()),
        ("length past the end", b"9:1".to_vec()),
        (
            "other version",
            frame(&[b"2", b"stage", b"0", b"r", b"a", b"0", b"t"]),
        ),
        (
            "phase not on the list",
            frame(&[b"1", b"publish", b"0", b"r", b"a", b"0", b"t"]),
        ),
        (
            "mark not on the list",
            frame(&[b"1", b"stage", b"2", b"r", b"a", b"0", b"t"]),
        ),
        (
            "registry not utf-8",
            frame(&[b"1", b"stage", b"0", b"\xff", b"a", b"0", b"t"]),
        ),
        (
            "actor not utf-8",
            frame(&[b"1", b"stage", b"0", b"r", b"\xff", b"0", b"t"]),
        ),
        (
            "count not a number",
            frame(&[b"1", b"stage", b"0", b"r", b"a", b"x", b"t"]),
        ),
        (
            "count past the bytes",
            frame(&[b"1", b"stage", b"0", b"r", b"a", b"4000000000", b"t"]),
        ),
        (
            "type not a number",
            frame(&[
                b"1", b"stage", b"0", b"r", b"a", b"1", b"c", b"x", b"n", b"v", b"p", b"t",
            ]),
        ),
        ("no token", frame(&[b"1", b"stage", b"0", b"r", b"a", b"0"])),
        ("empty token on a write", token_frame(b"")),
        ("token with a space", token_frame(b"a b")),
        ("token with a newline", token_frame(b"a\nb")),
        (
            "token on a read",
            frame(&[b"1", b"read", b"0", b"r", b"a", b"0", b"t"]),
        ),
    ];
    let mut trailing = good.clone();
    trailing.push(b'x');
    cases.push(("trailing bytes", trailing));
    cases.push(("truncated", good[..good.len() - 3].to_vec()));
    for (what, bytes) in cases {
        assert!(Envelope::decode(&bytes).is_err(), "{what}");
    }
    assert!(Envelope::decode(&good).is_ok());
}

#[test]
fn no_grant_is_enough_for_a_read_and_never_for_a_write() {
    let none = Grant::none();
    let set = set();
    let refs: Vec<&Coordinate> = set.iter().collect();
    for verb in READ_VERBS {
        assert!(authorise(&none, &call(verb, &refs)).unwrap().is_none());
    }
    for verb in WRITE_VERBS {
        assert_eq!(
            authorise(&none, &call(verb, &refs)).err(),
            Some(denied()),
            "{verb:?}"
        );
    }
}

#[test]
fn a_grant_for_the_call_is_accepted_and_hands_back_its_token() {
    let set = set();
    let refs: Vec<&Coordinate> = set.iter().collect();
    for (phase, verbs) in [
        (GrantPhase::Stage, [Verb::Stage, Verb::Abort]),
        (GrantPhase::Commit, [Verb::Commit, Verb::Yank]),
    ] {
        for verb in verbs {
            let g = grant(phase, set.clone(), NOW + 1);
            let got = authorise(&g, &call(verb, &refs)).unwrap().unwrap();
            assert_eq!(got.bearer_token(), Some(TOKEN), "{verb:?}");
            assert_eq!(got.actor, "ci:gh-repo:9001");
        }
    }
}

#[test]
fn a_write_verb_refuses_the_other_phase_and_a_read_grant() {
    let set = set();
    let refs: Vec<&Coordinate> = set.iter().collect();
    let cases = [
        (Verb::Stage, GrantPhase::Commit),
        (Verb::Abort, GrantPhase::Commit),
        (Verb::Commit, GrantPhase::Stage),
        (Verb::Yank, GrantPhase::Stage),
        (Verb::Stage, GrantPhase::Read),
        (Verb::Abort, GrantPhase::Read),
        (Verb::Commit, GrantPhase::Read),
        (Verb::Yank, GrantPhase::Read),
    ];
    for (verb, phase) in cases {
        let g = grant(phase, set.clone(), NOW + 1);
        assert_eq!(
            authorise(&g, &call(verb, &refs)).err(),
            Some(denied()),
            "{verb:?} with a {phase:?} grant"
        );
    }
}

#[test]
fn a_read_takes_a_grant_of_any_phase() {
    let set = set();
    let refs: Vec<&Coordinate> = set.iter().collect();
    for phase in [GrantPhase::Stage, GrantPhase::Commit, GrantPhase::Read] {
        for verb in READ_VERBS {
            let g = grant(phase, set.clone(), NOW + 1);
            assert!(authorise(&g, &call(verb, &refs)).unwrap().is_some());
        }
    }
}

#[test]
fn a_read_that_names_no_coordinate_needs_no_coverage() {
    let g = grant(GrantPhase::Stage, Vec::new(), NOW + 1);
    for verb in [Verb::Newest, Verb::List] {
        assert!(authorise(&g, &call(verb, &[])).unwrap().is_some());
    }
}

#[test]
fn a_grant_for_another_registry_is_denied_for_every_verb() {
    let set = set();
    let refs: Vec<&Coordinate> = set.iter().collect();
    for verb in READ_VERBS.into_iter().chain(WRITE_VERBS) {
        let phase = match verb {
            Verb::Commit | Verb::Yank => GrantPhase::Commit,
            _ => GrantPhase::Stage,
        };
        let g = grant(phase, set.clone(), NOW + 1);
        let mut other = call(verb, &refs);
        other.registry = "https://registry.example.com/api/v1/";
        assert_eq!(authorise(&g, &other).err(), Some(denied()), "{verb:?}");
    }
}

#[test]
fn one_uncovered_coordinate_is_enough_to_refuse_and_every_field_counts() {
    let held = set();
    let g = grant(GrantPhase::Stage, held.clone(), NOW + 1);
    let mut mutants = Vec::new();
    for at in 0..5 {
        let mut c = held[2].clone();
        match at {
            0 => c.channel = "other".to_owned(),
            1 => c.r#type += 1,
            2 => c.name = "gamma".to_owned(),
            3 => c.version = "0.1.1".to_owned(),
            _ => c.platform = "linux-64".to_owned(),
        }
        mutants.push(c);
    }
    for mutant in &mutants {
        // The first coordinate is covered; only the last is not.
        let asked = [&held[0], mutant];
        assert_eq!(
            authorise(&g, &call(Verb::Stage, &asked)).err(),
            Some(denied())
        );
        let observed = [&held[0], &held[1], mutant];
        assert_eq!(
            authorise(&g, &call(Verb::Observe, &observed)).err(),
            Some(denied())
        );
    }
}

#[test]
fn bytes_that_are_not_an_envelope_are_denied() {
    let junk = Grant::issued(b"not an envelope".to_vec(), NOW + 1);
    for verb in READ_VERBS.into_iter().chain(WRITE_VERBS) {
        assert_eq!(authorise(&junk, &call(verb, &[])).err(), Some(denied()));
    }
}

#[test]
fn a_grant_expiring_at_now_is_expired_and_one_instant_later_is_not() {
    let set = set();
    let refs: Vec<&Coordinate> = set.iter().collect();
    for verb in READ_VERBS.into_iter().chain([Verb::Stage]) {
        let at = grant(GrantPhase::Stage, set.clone(), NOW);
        assert_eq!(
            authorise(&at, &call(verb, &refs)).err(),
            Some(expired()),
            "{verb:?}"
        );
        let before = grant(GrantPhase::Stage, set.clone(), NOW - 1);
        assert_eq!(
            authorise(&before, &call(verb, &refs)).err(),
            Some(expired())
        );
        let after = grant(GrantPhase::Stage, set.clone(), NOW + 1);
        assert!(authorise(&after, &call(verb, &refs)).is_ok());
    }
}

#[test]
fn the_remaining_life_must_cover_the_budget_of_the_call() {
    let set = set();
    let refs: Vec<&Coordinate> = set.iter().collect();
    let g = grant(GrantPhase::Commit, set.clone(), NOW + 510_000);
    let mut commit = call(Verb::Commit, &refs);
    commit.lasts_ms = 509_999;
    assert!(authorise(&g, &commit).is_ok());
    commit.lasts_ms = 510_000;
    assert_eq!(authorise(&g, &commit).err(), Some(expired()));
    commit.lasts_ms = u64::MAX;
    assert_eq!(authorise(&g, &commit).err(), Some(expired()));
}

#[test]
fn a_grant_that_is_wrong_and_expired_is_denied_not_expired() {
    let set = set();
    let refs: Vec<&Coordinate> = set.iter().collect();
    let g = grant(GrantPhase::Commit, set.clone(), NOW - 1);
    assert_eq!(
        authorise(&g, &call(Verb::Stage, &refs)).err(),
        Some(denied())
    );
}

#[test]
fn mopms_answers_that_mean_a_bad_grant_are_classified() {
    let ceiling = Duration::from_secs(600);
    let denied_codes = ["token_unknown", "token_revoked", "token_missing"];
    for op in [Op::Read, Op::Create, Op::Yank] {
        for code in denied_codes {
            let r = response(401, &[], &error_body(code));
            assert_eq!(
                map_reply(op, &r, ceiling),
                Some(Mapped::Failed(denied())),
                "{op:?} {code}"
            );
        }
        let scope = response(403, &[], &error_body("scope_missing"));
        assert_eq!(
            map_reply(op, &scope, ceiling),
            Some(Mapped::Failed(denied()))
        );
        let unparsable = response(401, &[], "<html>");
        assert_eq!(
            map_reply(op, &unparsable, ceiling),
            Some(Mapped::Failed(denied()))
        );
    }
    let gone = response(401, &[], &error_body("token_expired"));
    for op in [Op::Read, Op::Create, Op::Yank] {
        assert_eq!(
            map_reply(op, &gone, ceiling),
            Some(Mapped::Failed(expired())),
            "{op:?}"
        );
    }
    // After a commit was sent, a 401 does not say it did not land.
    for op in [Op::Commit, Op::Poll] {
        let unknown = map_reply(op, &gone, ceiling);
        assert!(
            matches!(unknown, Some(Mapped::Failed(SeamError::Unknown(_)))),
            "{op:?}"
        );
    }
}

#[test]
fn the_server_failures_of_a_write_before_and_after_the_commit() {
    let ceiling = Duration::from_secs(600);
    let busy = response(503, &[], &error_body("unavailable"));
    assert!(matches!(
        map_reply(Op::Create, &busy, ceiling),
        Some(Mapped::Failed(SeamError::Transient(_)))
    ));
    for op in [Op::Commit, Op::Poll, Op::Yank] {
        assert!(
            matches!(
                map_reply(op, &busy, ceiling),
                Some(Mapped::Failed(SeamError::Unknown(_)))
            ),
            "{op:?}"
        );
    }
}
