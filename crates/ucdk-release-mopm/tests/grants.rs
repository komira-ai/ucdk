// SPDX-License-Identifier: Apache-2.0
//! The grant envelope and the client-side checks of a grant: every refusal sends nothing, and
//! the answers of mopm that mean a bad grant are classified.

mod common;

use std::time::Duration;

use common::{coordinate, error_body, response};
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_mopm::{Envelope, Malformed, Mapped, Op, Use, Verb, authorise, map_reply};
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
    .unwrap()
}

fn grant(phase: GrantPhase, coordinates: Vec<Coordinate>, expires_ms: u64) -> Grant {
    Grant::issued(envelope(phase, coordinates).encode().to_vec(), expires_ms)
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
        "https://r.example.com/x:y".to_owned(),
        "u:42".to_owned(),
        vec![tricky.clone(), coordinate("alpha", "1.2.3", "noarch")],
        TOKEN.to_vec(),
    )
    .unwrap();
    let back = Envelope::decode(&original.encode()).unwrap();
    assert_eq!(back.phase, GrantPhase::Commit);
    assert!(back.exceptional);
    assert_eq!(back.registry, "https://r.example.com/x:y");
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
fn bytes_that_are_not_an_envelope_are_malformed_and_name_the_part_at_fault() {
    let good = envelope(GrantPhase::Stage, set()).encode();
    let head: [&[u8]; 5] = [b"1", b"stage", b"0", b"r", b"a"];
    let with = |tail: &[&[u8]]| {
        let parts: Vec<&[u8]> = head.iter().copied().chain(tail.iter().copied()).collect();
        frame(&parts)
    };
    let one = |field: usize, value: &'static [u8]| {
        let mut c: [&[u8]; 5] = [b"c", b"1", b"n", b"v", b"p"];
        c[field] = value;
        with(&[b"1", c[0], c[1], c[2], c[3], c[4], b"t"])
    };
    let mut cases: Vec<(&str, Vec<u8>, Malformed)> = vec![
        ("empty", Vec::new(), Malformed::Frame),
        ("no colon", b"11".to_vec(), Malformed::Frame),
        ("length not utf-8", b"\xff:x".to_vec(), Malformed::Frame),
        ("length not a number", b"x:x".to_vec(), Malformed::Frame),
        ("signed length", b"+1:1".to_vec(), Malformed::Frame),
        ("leading zero", b"01:1".to_vec(), Malformed::Frame),
        ("length past the end", b"9:1".to_vec(), Malformed::Frame),
        (
            "length past usize",
            b"99999999999999999999999:1".to_vec(),
            Malformed::Frame,
        ),
        (
            "other version",
            frame(&[b"2", b"stage", b"0", b"r", b"a", b"0", b"t"]),
            Malformed::Version,
        ),
        (
            "phase not on the list",
            frame(&[b"1", b"publish", b"0", b"r", b"a", b"0", b"t"]),
            Malformed::Phase,
        ),
        (
            "mark not on the list",
            frame(&[b"1", b"stage", b"2", b"r", b"a", b"0", b"t"]),
            Malformed::Mark,
        ),
        (
            "registry not utf-8",
            frame(&[b"1", b"stage", b"0", b"\xff", b"a", b"0", b"t"]),
            Malformed::Text,
        ),
        (
            "actor not utf-8",
            frame(&[b"1", b"stage", b"0", b"r", b"\xff", b"0", b"t"]),
            Malformed::Text,
        ),
        ("channel not utf-8", one(0, b"\xff"), Malformed::Text),
        ("type not utf-8", one(1, b"\xff"), Malformed::Text),
        ("name not utf-8", one(2, b"\xff"), Malformed::Text),
        ("version not utf-8", one(3, b"\xff"), Malformed::Text),
        ("platform not utf-8", one(4, b"\xff"), Malformed::Text),
        ("count not a number", with(&[b"x", b"t"]), Malformed::Number),
        (
            "count with a leading zero",
            with(&[b"00", b"t"]),
            Malformed::Number,
        ),
        ("count with a sign", with(&[b"+0", b"t"]), Malformed::Number),
        ("negative count", with(&[b"-1", b"t"]), Malformed::Number),
        (
            "count past usize",
            with(&[b"18446744073709551616", b"t"]),
            Malformed::Number,
        ),
        (
            "count past the bytes",
            with(&[b"4000000000", b"t"]),
            Malformed::Frame,
        ),
        ("type not a number", one(1, b"x"), Malformed::Number),
        ("type with a leading zero", one(1, b"01"), Malformed::Number),
        ("type with a plus sign", one(1, b"+1"), Malformed::Number),
        ("type minus zero", one(1, b"-0"), Malformed::Number),
        ("type past i32", one(1, b"2147483648"), Malformed::Number),
        ("type below i32", one(1, b"-2147483649"), Malformed::Number),
        ("no token", with(&[b"0"]), Malformed::Frame),
        (
            "empty token on a write",
            with(&[b"0", b""]),
            Malformed::Token,
        ),
        (
            "token with a space",
            with(&[b"0", b"a b"]),
            Malformed::Token,
        ),
        (
            "token with a newline",
            with(&[b"0", b"a\nb"]),
            Malformed::Token,
        ),
        (
            "token not ascii",
            with(&[b"0", "\u{e9}".as_bytes()]),
            Malformed::Token,
        ),
        (
            "token on a read",
            frame(&[b"1", b"read", b"0", b"r", b"a", b"0", b"t"]),
            Malformed::Token,
        ),
    ];
    let mut trailing = good.to_vec();
    trailing.push(b'x');
    cases.push(("trailing bytes", trailing, Malformed::Trailing));
    cases.push((
        "truncated",
        good[..good.len() - 3].to_vec(),
        Malformed::Frame,
    ));
    for (what, bytes, fault) in cases {
        assert_eq!(Envelope::decode(&bytes).err(), Some(fault), "{what}");
    }
    assert!(Envelope::decode(&good).is_ok());
}

#[test]
fn a_type_at_either_end_of_its_range_and_unicode_texts_round_trip() {
    for r#type in [i32::MIN, -1, 0, i32::MAX] {
        let c = Coordinate {
            channel: "\u{e9}t\u{e9}".to_owned(),
            r#type,
            name: "\u{1f600}".to_owned(),
            version: "1.0".to_owned(),
            platform: "\u{4e2d}".to_owned(),
        };
        let original = Envelope::new(
            GrantPhase::Stage,
            false,
            "https://r\u{e9}.example.com".to_owned(),
            "u:\u{e9}".to_owned(),
            vec![c.clone()],
            TOKEN.to_vec(),
        )
        .unwrap();
        let back = Envelope::decode(&original.encode()).unwrap();
        assert_eq!(back.coordinates, [c], "{type}");
        assert_eq!(back.registry, "https://r\u{e9}.example.com");
        assert_eq!(back.actor, "u:\u{e9}");
    }
}

#[test]
fn an_envelope_whose_token_does_not_fit_its_phase_cannot_be_made() {
    let make = |phase, token: &[u8]| {
        Envelope::new(
            phase,
            false,
            REGISTRY.to_owned(),
            "u:1".to_owned(),
            Vec::new(),
            token.to_vec(),
        )
        .err()
    };
    assert_eq!(make(GrantPhase::Read, b"t"), Some(Malformed::Token));
    for phase in [GrantPhase::Stage, GrantPhase::Commit] {
        assert_eq!(make(phase, b""), Some(Malformed::Token));
        assert_eq!(make(phase, b"a b"), Some(Malformed::Token));
        assert_eq!(make(phase, b"t"), None);
    }
    assert_eq!(make(GrantPhase::Read, b""), None);
}

#[test]
fn a_malformed_envelope_says_which_part_and_nothing_of_its_bytes() {
    let cases = [
        (Malformed::Frame, "a length frame"),
        (Malformed::Version, "the version"),
        (Malformed::Phase, "the phase"),
        (Malformed::Mark, "the exceptional mark"),
        (Malformed::Text, "a text field"),
        (Malformed::Number, "a number"),
        (Malformed::Token, "the token"),
        (Malformed::Trailing, "the bytes after the token"),
    ];
    for (fault, part) in cases {
        assert_eq!(
            fault.to_string(),
            format!("not a grant envelope: {part} is malformed")
        );
        let error: &dyn std::error::Error = &fault;
        assert!(error.source().is_none());
    }
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
    let mut changed = Vec::new();
    for at in 0..5 {
        let mut c = held[2].clone();
        match at {
            0 => c.channel = "other".to_owned(),
            1 => c.r#type += 1,
            2 => c.name = "gamma".to_owned(),
            3 => c.version = "0.1.1".to_owned(),
            _ => c.platform = "linux-64".to_owned(),
        }
        changed.push(c);
    }
    for changed in &changed {
        // The first coordinate is covered; only the last is not.
        let asked = [&held[0], changed];
        assert_eq!(
            authorise(&g, &call(Verb::Stage, &asked)).err(),
            Some(denied())
        );
        let observed = [&held[0], &held[1], changed];
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
    // Of the wrong phase.
    let g = grant(GrantPhase::Commit, set.clone(), NOW - 1);
    assert_eq!(
        authorise(&g, &call(Verb::Stage, &refs)).err(),
        Some(denied())
    );
    // For another registry.
    let stage = grant(GrantPhase::Stage, set.clone(), NOW - 1);
    let mut elsewhere = call(Verb::Stage, &refs);
    elsewhere.registry = "https://other.example.com/api/v1";
    assert_eq!(authorise(&stage, &elsewhere).err(), Some(denied()));
    // Leaving a coordinate uncovered.
    let held = grant(GrantPhase::Stage, set[..2].to_vec(), NOW - 1);
    assert_eq!(
        authorise(&held, &call(Verb::Stage, &refs)).err(),
        Some(denied())
    );
    // Each of them alone, unexpired, is still refused, and the same grant right is expired.
    let fresh = grant(GrantPhase::Stage, set[..2].to_vec(), NOW + 1);
    assert_eq!(
        authorise(&fresh, &call(Verb::Stage, &refs)).err(),
        Some(denied())
    );
    assert_eq!(
        authorise(&held, &call(Verb::Stage, &refs[..2])).err(),
        Some(expired())
    );
}

#[test]
fn a_call_may_name_any_subset_of_what_the_grant_covers_in_any_order() {
    let set = set();
    let g = grant(GrantPhase::Stage, set.clone(), NOW + 1);
    let orders: [&[usize]; 6] = [&[2, 1, 0], &[1, 0, 2], &[2], &[1], &[2, 0], &[0, 0, 2]];
    for order in orders {
        let asked: Vec<&Coordinate> = order.iter().map(|i| &set[*i]).collect();
        for verb in [Verb::Stage, Verb::Abort, Verb::Observe, Verb::Fetch] {
            assert!(
                authorise(&g, &call(verb, &asked)).unwrap().is_some(),
                "{order:?} {verb:?}"
            );
        }
    }
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
