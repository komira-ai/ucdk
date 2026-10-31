// SPDX-License-Identifier: Apache-2.0
//! The record codec: every kind round-trips, the payload keys are a closed list, and each kind
//! carries the fields the design gives it. What a reader refuses is in `codec_refusals`.

mod codec_records;

use std::collections::BTreeSet;

use codec_records::{
    action, every_record, grant, grant_with, publication, publication_of, run, seal, seal_of,
    stage, stage_of, validation, written,
};
use ucdk_release::codec::{
    CodecError, GrantAnswer, GrantFault, GrantPhase, Phase, Publication, Record, Seal, Stage,
    coordinate_text, keys_of,
};
use ucdk_release::transitions::Kind;
use ucdk_release_proto::v1::{ArtifactType, Coordinate};

#[test]
fn every_record_kind_round_trips() {
    let all = every_record();
    let kinds: BTreeSet<&str> = all.iter().map(|r| r.kind().tag()).collect();
    let listed: BTreeSet<&str> = Kind::ALL.iter().map(|k| k.tag()).collect();
    assert_eq!(
        kinds, listed,
        "a kind is written by the codec and listed, or neither"
    );
    for record in all {
        let encoded = written(&record);
        assert_eq!(encoded.kind.as_str(), record.kind().tag());
        assert_eq!(Record::decode(&encoded), Ok(record.clone()), "{record:?}");
    }
}

#[test]
fn payload_keys_are_a_closed_list() {
    // The list is the format: a key added or renamed changes what every reader accepts. It has
    // no key that could hold a credential.
    let expected: [(Kind, &[&str]); 7] = [
        (
            Kind::Run,
            &[
                "machine",
                "revision",
                "version",
                "build",
                "commit",
                "timestamp",
            ],
        ),
        (
            Kind::Stage,
            &["attempt", "platform", "set", "generation", "confirmed"],
        ),
        (Kind::Action, &["input", "set", "generation"]),
        (
            Kind::Publication,
            &[
                "digest",
                "build",
                "commit",
                "exceptional",
                "run",
                "stage",
                "attempt",
                "phase",
            ],
        ),
        (Kind::Validation, &["set"]),
        (
            Kind::Grant,
            &[
                "phase",
                "channel",
                "repository",
                "subject",
                "audience",
                "scope",
                "coordinates",
                "outcome",
                "fault",
                "expires_ms",
            ],
        ),
        (
            Kind::Seal,
            &[
                "set",
                "generation",
                "inputs",
                "environment",
                "machine",
                "version",
                "build",
                "commit",
                "timestamp",
                "run",
                "attempt",
            ],
        ),
    ];
    assert_eq!(expected.map(|(k, _)| k), Kind::ALL);
    for (kind, keys) in expected {
        assert_eq!(keys_of(kind), keys, "{kind:?}");
        // No kind has a key that names a secret.
        for key in keys {
            for secret in ["token", "secret", "password", "credential", "jwt", "key"] {
                assert!(!key.contains(secret), "{kind:?} has the key {key}");
            }
        }
    }
    // A record writes only keys of its kind.
    for record in every_record() {
        let encoded = written(&record);
        let allowed = keys_of(record.kind());
        assert!(
            encoded
                .outputs
                .keys()
                .all(|k| allowed.contains(&k.as_str())),
            "{encoded:?}"
        );
    }
    // The fullest form of each kind writes all of its keys, except a grant, whose answer is
    // granted or refused: the two forms of it together write all of them.
    for record in [
        run(),
        stage(true),
        action(true),
        publication(true, Some(Phase::Staging)),
        validation(true),
        seal(),
    ] {
        let encoded = written(&record);
        let keys: BTreeSet<&str> = encoded.outputs.keys().map(String::as_str).collect();
        let allowed: BTreeSet<&str> = keys_of(record.kind()).iter().copied().collect();
        assert_eq!(keys, allowed, "{}", encoded.kind);
    }
    let granted = written(&grant());
    let refused = written(&grant_with(
        GrantPhase::Commit,
        "push",
        Some(GrantAnswer::Refused {
            fault: GrantFault::Used,
        }),
        1,
        1,
    ));
    let union: BTreeSet<&str> = granted
        .outputs
        .keys()
        .chain(refused.outputs.keys())
        .map(String::as_str)
        .collect();
    assert_eq!(
        union,
        keys_of(Kind::Grant)
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
    );
    // A reader refuses a key that is not on the list, whatever the kind, and the refusal names
    // it.
    for record in every_record() {
        let mut encoded = written(&record);
        encoded
            .outputs
            .insert("token".to_owned(), "secret".to_owned());
        let err = Record::decode(&encoded).expect_err("an unknown key");
        assert!(
            matches!(&err, CodecError::UnknownKey { key, .. } if key == "token"),
            "{err:?}"
        );
        assert!(err.to_string().contains("token"), "{err}");
    }
}

#[test]
fn publication_record_carries_run_stage_and_attempt() {
    let encoded = written(&publication(true, None));
    assert_eq!(encoded.kind.as_str(), "publication");
    assert_eq!(encoded.name, "stable/file/tool/1.2.3+build.4/linux-x86_64");
    assert_eq!(encoded.physical, None);
    assert_eq!(encoded.outputs["run"], "run-7");
    assert_eq!(encoded.outputs["stage"], "publish");
    assert_eq!(encoded.outputs["attempt"], "3");
    assert_eq!(encoded.outputs["exceptional"], "1");
    assert_eq!(encoded.outputs["digest"], "4".repeat(64));
    assert_eq!(encoded.outputs["build"], "41");
    assert_eq!(encoded.outputs["commit"], "abc123");
    // The mark is `1` or absent, never `0`.
    assert!(
        !written(&publication(false, None))
            .outputs
            .contains_key("exceptional")
    );
}

#[test]
fn publication_record_carries_its_phase() {
    for (phase, word) in [
        (Phase::Staging, "staging"),
        (Phase::Committing, "committing"),
    ] {
        let encoded = written(&publication(false, Some(phase)));
        assert_eq!(encoded.outputs["phase"], word);
        let back = publication_of(Record::decode(&encoded).expect("a publication"));
        assert_eq!(back.phase, Some(phase));
    }
    // A confirmed record has no phase: the key is absent.
    assert!(
        !written(&publication(false, None))
            .outputs
            .contains_key("phase")
    );
    // Only the two phases of a publication are words.
    for word in ["", "stage", "commit", "Staging", "committed"] {
        let mut encoded = written(&publication(false, Some(Phase::Staging)));
        encoded.outputs.insert("phase".to_owned(), word.to_owned());
        let got = Record::decode(&encoded);
        assert!(
            matches!(&got, Err(CodecError::Bad { key, value, .. }) if key == "phase" && value == word),
            "{word:?}: {got:?}"
        );
    }
}

#[test]
fn stage_record_carries_attempt_platform_and_generation() {
    let encoded = written(&stage(true));
    assert_eq!(encoded.name, "build");
    assert_eq!(encoded.outputs["attempt"], "2");
    assert_eq!(encoded.outputs["platform"], "linux-x86_64");
    assert_eq!(encoded.outputs["generation"], "run-7-a2");
    assert_eq!(encoded.outputs["set"], "2".repeat(64));
    assert_eq!(encoded.outputs["confirmed"], "1");
    let light = written(&stage(false));
    assert!(!light.outputs.contains_key("set"));
    assert!(!light.outputs.contains_key("generation"));
    assert!(!light.outputs.contains_key("confirmed"));
}

#[test]
fn every_record_kind_has_a_golden() {
    // One record of each kind as the codec writes it, byte for byte: the path, the name and
    // every key and value in key order. The text is the format; a second implementation that
    // writes it reads what this one wrote. A change to any line is a change of format.
    let records = [
        run(),
        stage(true),
        action(true),
        publication(true, Some(Phase::Committing)),
        validation(true),
        grant(),
        seal(),
    ];
    let kinds: BTreeSet<&str> = records.iter().map(|r| r.kind().tag()).collect();
    assert_eq!(kinds.len(), Kind::ALL.len(), "a golden for every kind");
    let mut text = String::new();
    for record in &records {
        let encoded = written(record);
        text.push_str(&format!(
            "{} {}\n  name {}\n",
            encoded.kind,
            encoded.path().expect("a path"),
            encoded.name
        ));
        for (key, value) in &encoded.outputs {
            text.push_str(&format!("  {key} = {value}\n"));
        }
    }
    assert_eq!(text, include_str!("codec_golden.txt"));
}

#[test]
fn a_stage_without_a_platform_round_trips() {
    // A Test-only stage, and a `CHANGE_CHECK` one, name no platform (2): the key is absent, and
    // the record reads back with none, whatever else it carries.
    for full in [false, true] {
        let platformless = Stage {
            platform: None,
            ..stage_of(stage(full))
        };
        let record = Record::Stage(platformless);
        let encoded = written(&record);
        assert!(!encoded.outputs.contains_key("platform"), "{encoded:?}");
        assert_eq!(Record::decode(&encoded), Ok(record), "full: {full}");
    }
}

#[test]
fn action_record_carries_generation() {
    let encoded = written(&action(true));
    assert_eq!(encoded.kind.as_str(), "action");
    assert_eq!(encoded.name, "build/compile");
    assert_eq!(encoded.outputs["generation"], "run-7-a2");
    assert_eq!(encoded.outputs["input"], "3".repeat(64));
    assert_eq!(encoded.outputs["set"], "2".repeat(64));
    assert!(!written(&action(false)).outputs.contains_key("generation"));
}

#[test]
fn the_validation_record_names_its_stage_and_validation_and_holds_only_the_set() {
    let encoded = written(&validation(true));
    assert_eq!(encoded.name, "build/closure");
    assert_eq!(encoded.outputs.len(), 1);
    assert_eq!(encoded.outputs["set"], "2".repeat(64));
    // A read-back reads a channel, not a set.
    assert!(written(&validation(false)).outputs.is_empty());
}

#[test]
fn grant_record_carries_no_secret_key() {
    // A grant with every field set, and nothing in it that could be a token.
    let encoded = written(&grant());
    assert_eq!(encoded.kind.as_str(), "grant");
    assert_eq!(encoded.name, "publish/push/stage/a2-1");
    assert_eq!(encoded.physical, None);
    assert_eq!(encoded.outputs["phase"], "stage");
    assert_eq!(encoded.outputs["channel"], "stable");
    assert_eq!(encoded.outputs["repository"], "file");
    assert_eq!(
        encoded.outputs["subject"],
        "repo:example/project:ref:refs/heads/main"
    );
    assert_eq!(encoded.outputs["audience"], "registry");
    assert_eq!(encoded.outputs["scope"], "stage");
    assert_eq!(encoded.outputs["coordinates"], "6".repeat(64));
    assert_eq!(encoded.outputs["outcome"], "granted");
    assert_eq!(encoded.outputs["expires_ms"], "1700000600000");
    assert!(!encoded.outputs.contains_key("fault"));
    let listed = keys_of(Kind::Grant);
    assert!(encoded.outputs.keys().all(|k| listed.contains(&k.as_str())));
    // A payload that tries to add a key for a token, an identity token or an issuer's token id
    // is refused, whatever the key is called.
    for key in [
        "token",
        "identity_token",
        "id_token",
        "jti",
        "access_token",
        "claims",
        "secret",
    ] {
        let mut with = encoded.clone();
        with.outputs.insert(key.to_owned(), "eyJ".to_owned());
        let got = Record::decode(&with);
        assert!(
            matches!(&got, Err(CodecError::UnknownKey { kind: "grant", key: k }) if k == key),
            "{key}: {got:?}"
        );
    }
    // A refused exchange holds its fault and no expiry.
    let refused = written(&grant_with(
        GrantPhase::Stage,
        "push",
        Some(GrantAnswer::Refused {
            fault: GrantFault::Expired,
        }),
        1,
        1,
    ));
    assert_eq!(refused.outputs["outcome"], "refused");
    assert_eq!(refused.outputs["fault"], "expired");
    assert!(!refused.outputs.contains_key("expires_ms"));
    // An exchange whose answer was lost holds neither.
    let intent = written(&grant_with(GrantPhase::Stage, "push", None, 1, 1));
    assert!(!intent.outputs.contains_key("outcome"));
    assert!(!intent.outputs.contains_key("fault"));
    assert!(!intent.outputs.contains_key("expires_ms"));
}

#[test]
fn grant_record_carries_the_read_phase_and_the_validation_holder_name() {
    let read = grant_with(
        GrantPhase::Read,
        "validation:readback",
        Some(GrantAnswer::Granted { expires_ms: 5 }),
        3,
        2,
    );
    let encoded = written(&read);
    assert_eq!(encoded.name, "publish/validation:readback/read/a3-2");
    assert_eq!(encoded.outputs["phase"], "read");
    assert_eq!(Record::decode(&encoded), Ok(read));
    // The holder of a Smoke action is its own name; the two never collide because a name has no
    // `:`.
    let smoke = written(&grant_with(GrantPhase::Read, "smoke", None, 3, 2));
    assert_eq!(smoke.name, "publish/smoke/read/a3-2");
    for (phase, word) in [
        (GrantPhase::Stage, "stage"),
        (GrantPhase::Commit, "commit"),
        (GrantPhase::Read, "read"),
    ] {
        let encoded = written(&grant_with(phase, "push", None, 1, 7));
        assert_eq!(encoded.name, format!("publish/push/{word}/a1-7"));
        assert_eq!(encoded.outputs["phase"], word);
    }
}

#[test]
fn attempt_zero_is_refused() {
    // Attempts count from 1. A stage, a publication, a grant (in its name) and a seal each carry
    // one, and the codec neither writes nor reads a 0.
    for record in [stage(true), publication(true, None), seal()] {
        let mut encoded = written(&record);
        encoded.outputs.insert("attempt".to_owned(), "0".to_owned());
        let got = Record::decode(&encoded);
        assert!(
            matches!(&got, Err(CodecError::Bad { key, value, .. }) if key == "attempt" && value == "0"),
            "{got:?}"
        );
        encoded.outputs.insert("attempt".to_owned(), "1".to_owned());
        assert!(Record::decode(&encoded).is_ok());
    }
    let mut encoded = written(&grant());
    encoded.name = "publish/push/stage/a0-1".to_owned();
    let got = Record::decode(&encoded);
    assert!(
        matches!(&got, Err(CodecError::Bad { key, value, .. }) if key == "attempt" && value == "0"),
        "{got:?}"
    );
    // The writer refuses what the reader refuses.
    let mut zero_publication = publication_of(publication(false, None));
    zero_publication.attempt = 0;
    for record in [
        Record::Stage(Stage {
            attempt: 0,
            ..stage_of(stage(true))
        }),
        Record::Seal(Seal {
            attempt: 0,
            ..seal_of(seal())
        }),
        grant_with(GrantPhase::Stage, "push", None, 0, 1),
        Record::Publication(zero_publication),
    ] {
        let err = record.encode().expect_err("attempt 0");
        assert!(
            matches!(&err, CodecError::Bad { key, value, .. } if key == "attempt" && value == "0"),
            "{err:?}"
        );
    }
}

fn coordinate(channel: &str, name: &str, version: &str, platform: &str) -> Coordinate {
    Coordinate {
        channel: channel.to_owned(),
        r#type: ArtifactType::File as i32,
        name: name.to_owned(),
        version: version.to_owned(),
        platform: platform.to_owned(),
    }
}

#[test]
fn a_publication_coordinate_is_framed_so_slashes_cannot_alias() {
    // The text is each field's length in bytes, a `:` and the field, in the order channel,
    // type, name, version, platform: no joining character, so none can be moved from one field
    // to the next.
    assert_eq!(
        coordinate_text(&coordinate(
            "stable",
            "tool",
            "1.2.3+build.4",
            "linux-x86_64"
        )),
        "6:stable1:14:tool13:1.2.3+build.412:linux-x86_64"
    );
    // Two coordinates that a `/` join would spell alike are two texts, and two records.
    let one = coordinate("stable", "a/b", "c", "p");
    let other = coordinate("stable", "a", "b/c", "p");
    assert_ne!(coordinate_text(&one), coordinate_text(&other));
    let channel_runs_on = coordinate("a/stable", "b", "c", "p");
    let name_runs_back = coordinate("a", "stable/b", "c", "p");
    assert_ne!(
        coordinate_text(&channel_runs_on),
        coordinate_text(&name_runs_back)
    );
    let publication = |c: &Coordinate| {
        let Record::Publication(p) = publication(false, None) else {
            panic!("a publication")
        };
        Record::Publication(Publication {
            coordinate: coordinate_text(c),
            ..p
        })
    };
    let (a, b) = (publication(&one), publication(&other));
    let (ea, eb) = (written(&a), written(&b));
    assert_ne!(
        ea.path(),
        eb.path(),
        "two coordinates are two ledger records"
    );
    // And whatever the fields hold, the record reads back the coordinate it was given.
    assert_eq!(Record::decode(&ea), Ok(a));
    assert_eq!(Record::decode(&eb), Ok(b));
}
