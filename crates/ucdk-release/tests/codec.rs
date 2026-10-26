// SPDX-License-Identifier: Apache-2.0
//! The record codec: every kind round-trips, the payload keys are a closed list, and a reader
//! refuses what the writer would not write.

use std::collections::BTreeSet;

use ucdk_core::{Hash256, Locator, RecordKey, Tag};
use ucdk_release::codec::{
    CodecError, Created, Encoded, KINDS, Publication, Record, Run, Seal, Stage, Step, Validation,
    keys_of,
};

fn hash(c: char) -> Hash256 {
    Hash256::from_hex(&c.to_string().repeat(64)).expect("a hash")
}

fn run() -> Record {
    Record::Run(Run {
        run_id: "run-7".to_owned(),
        machine: hash('1'),
        revision: "0123456789abcdef".to_owned(),
        version: "1.2.3".to_owned(),
        build: 41,
        commit: "abc123".to_owned(),
        timestamp: 1_700_000_000_000,
    })
}

fn stage(full: bool) -> Record {
    Record::Stage(Stage {
        stage: "build".to_owned(),
        attempt: 2,
        platform: "linux-x86_64".to_owned(),
        set: full.then(|| hash('2')),
        generation: full.then(|| "run-7-a2".to_owned()),
        confirmed: full,
    })
}

fn step(full: bool) -> Record {
    Record::Step(Step {
        stage: "build".to_owned(),
        step: "compile".to_owned(),
        input: hash('3'),
        set: full.then(|| hash('2')),
        generation: full.then(|| "run-7-a2".to_owned()),
    })
}

fn publication(exceptional: bool) -> Record {
    Record::Publication(Publication {
        coordinate: "stable/file/tool/1.2.3+build.4/linux-x86_64".to_owned(),
        digest: hash('4'),
        build: 41,
        commit: "abc123".to_owned(),
        exceptional,
        run: "run-7".to_owned(),
        stage: "publish".to_owned(),
        attempt: 3,
    })
}

fn validation(full: bool) -> Record {
    Record::Validation(Validation {
        stage: "build".to_owned(),
        validation: "closure".to_owned(),
        validation_run_id: "vr-0123456789ab-k1".to_owned(),
        k: 1,
        set: full.then(|| hash('2')),
    })
}

fn created() -> Record {
    Record::Created(Created {
        kind: "object-store".to_owned(),
        id: "objects/one two".to_owned(),
        validation_run_id: "vr-0123456789ab-k1".to_owned(),
    })
}

fn seal() -> Record {
    Record::Seal(Seal {
        stage: "build".to_owned(),
        platform: "linux-x86_64".to_owned(),
        revision: "0123456789abcdef".to_owned(),
        set: hash('2'),
        generation: "run-7-a2".to_owned(),
        inputs: hash('5'),
        environment: "build".to_owned(),
        machine: "demo".to_owned(),
        version: "1.2.3".to_owned(),
        build: 41,
        commit: "abc123".to_owned(),
        timestamp: 1_700_000_000_000,
        run: "run-7".to_owned(),
        attempt: 2,
    })
}

/// One of each kind, fully written, and the lighter forms of those that have optional keys.
fn every_record() -> Vec<Record> {
    vec![
        run(),
        stage(true),
        stage(false),
        step(true),
        step(false),
        publication(true),
        publication(false),
        validation(true),
        validation(false),
        created(),
        seal(),
    ]
}

fn kind_of(record: &Record) -> String {
    record.encode().kind.to_string()
}

#[test]
fn every_record_kind_round_trips() {
    let all = every_record();
    let kinds: BTreeSet<String> = all.iter().map(kind_of).collect();
    let listed: BTreeSet<String> = KINDS.iter().map(|(k, _)| (*k).to_owned()).collect();
    assert_eq!(
        kinds, listed,
        "a kind is written by the codec and listed, or neither"
    );
    for record in all {
        let written = record.encode();
        assert_eq!(Record::decode(&written), Ok(record.clone()), "{record:?}");
    }
}

#[test]
fn payload_keys_are_a_closed_list() {
    // The list is the format: a key added or renamed changes what every reader accepts. It has
    // no key that could hold a credential.
    let expected: [(&str, &[&str]); 7] = [
        (
            "run",
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
            "stage",
            &["attempt", "platform", "set", "generation", "confirmed"],
        ),
        ("step", &["input", "set", "generation"]),
        (
            "publication",
            &[
                "digest",
                "build",
                "commit",
                "exceptional",
                "run",
                "stage",
                "attempt",
            ],
        ),
        ("validation", &["k", "set"]),
        ("created", &["vr"]),
        (
            "seal",
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
    assert_eq!(KINDS, expected);
    // A record writes only keys of its kind, and the fullest form of each writes all of them.
    for record in every_record() {
        let written = record.encode();
        let allowed = keys_of(written.kind.as_str()).expect("a listed kind");
        assert!(
            written
                .outputs
                .keys()
                .all(|k| allowed.contains(&k.as_str())),
            "{written:?}"
        );
    }
    for record in [
        run(),
        stage(true),
        step(true),
        publication(true),
        validation(true),
        created(),
        seal(),
    ] {
        let written = record.encode();
        let keys: BTreeSet<&str> = written.outputs.keys().map(String::as_str).collect();
        let allowed: BTreeSet<&str> = keys_of(written.kind.as_str())
            .expect("kind")
            .iter()
            .copied()
            .collect();
        assert_eq!(keys, allowed, "{}", written.kind);
    }
    assert_eq!(keys_of("ledger"), None);
    // A reader refuses a key that is not on the list, whatever the kind.
    for record in every_record() {
        let mut written = record.encode();
        written
            .outputs
            .insert("token".to_owned(), "secret".to_owned());
        let err = Record::decode(&written).expect_err("an unknown key");
        assert!(
            matches!(&err, CodecError::UnknownKey { key, .. } if key == "token"),
            "{err:?}"
        );
        assert!(err.to_string().contains("token"), "{err}");
    }
}

#[test]
fn publication_record_carries_run_stage_and_attempt() {
    let written = publication(true).encode();
    assert_eq!(written.kind.as_str(), "publication");
    assert_eq!(written.name, "stable/file/tool/1.2.3+build.4/linux-x86_64");
    assert_eq!(written.physical, None);
    assert_eq!(written.outputs["run"], "run-7");
    assert_eq!(written.outputs["stage"], "publish");
    assert_eq!(written.outputs["attempt"], "3");
    assert_eq!(written.outputs["exceptional"], "1");
    assert_eq!(written.outputs["digest"], "4".repeat(64));
    assert_eq!(written.outputs["build"], "41");
    assert_eq!(written.outputs["commit"], "abc123");
    // The mark is `1` or absent, never `0`.
    assert!(
        !publication(false)
            .encode()
            .outputs
            .contains_key("exceptional")
    );
}

#[test]
fn stage_record_carries_attempt_platform_and_generation() {
    let written = stage(true).encode();
    assert_eq!(written.name, "build");
    assert_eq!(written.outputs["attempt"], "2");
    assert_eq!(written.outputs["platform"], "linux-x86_64");
    assert_eq!(written.outputs["generation"], "run-7-a2");
    assert_eq!(written.outputs["set"], "2".repeat(64));
    assert_eq!(written.outputs["confirmed"], "1");
    let light = stage(false).encode();
    assert!(!light.outputs.contains_key("set"));
    assert!(!light.outputs.contains_key("generation"));
    assert!(!light.outputs.contains_key("confirmed"));
}

#[test]
fn step_record_carries_generation() {
    let written = step(true).encode();
    assert_eq!(written.name, "build/compile");
    assert_eq!(written.outputs["generation"], "run-7-a2");
    assert_eq!(written.outputs["input"], "3".repeat(64));
    assert_eq!(written.outputs["set"], "2".repeat(64));
    assert!(!step(false).encode().outputs.contains_key("generation"));
}

#[test]
fn the_kinds_with_a_locator_write_it_and_the_rest_write_none() {
    assert_eq!(
        validation(true).encode().physical,
        Some(Locator::new("vr-0123456789ab-k1"))
    );
    assert_eq!(
        created().encode().physical,
        Some(Locator::new("objects/one two"))
    );
    assert_eq!(created().encode().name, "object-store/objects/one two");
    for record in [run(), stage(true), step(true), publication(true), seal()] {
        assert_eq!(record.encode().physical, None);
    }
    assert_eq!(seal().encode().name, "build/linux-x86_64/0123456789abcdef");
}

#[test]
fn a_record_path_is_its_tag_and_the_key_of_its_name() {
    let written = publication(true).encode();
    let path = written.path().expect("a path");
    let key = RecordKey::of_name(&written.name);
    assert_eq!(path.as_str(), format!("publication/k{key}"));
    assert_eq!(path.owner().as_str(), "publication");
    // The name keeps the characters a path segment cannot, and the path is still valid; another
    // name is another path.
    let mut other = written.clone();
    other.name = "stable/file/tool/1.2.4/linux-x86_64".to_owned();
    assert_ne!(other.path().expect("path"), path);
    for record in every_record() {
        record
            .encode()
            .path()
            .expect("every record has a valid path");
    }
    let bad_tag = Encoded {
        kind: Tag::new("not a segment"),
        name: "x".to_owned(),
        physical: None,
        outputs: Default::default(),
    };
    assert!(bad_tag.path().is_err());
}

#[test]
fn a_kind_that_is_not_listed_is_refused() {
    let mut written = run().encode();
    written.kind = Tag::new("deploy");
    let err = Record::decode(&written).expect_err("unknown kind");
    assert_eq!(err, CodecError::UnknownKind("deploy".to_owned()));
    assert!(err.to_string().contains("deploy"));
}

#[test]
fn a_required_key_that_is_missing_is_refused() {
    let optional: [(&str, &[&str]); 4] = [
        ("stage", &["set", "generation", "confirmed"]),
        ("step", &["set", "generation"]),
        ("publication", &["exceptional"]),
        ("validation", &["set"]),
    ];
    for record in [
        run(),
        stage(true),
        step(true),
        publication(true),
        validation(true),
        created(),
        seal(),
    ] {
        let written = record.encode();
        let kind = written.kind.to_string();
        let skippable: &[&str] = optional
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(&[], |(_, o)| *o);
        for key in keys_of(&kind).expect("kind") {
            let mut cut = written.clone();
            cut.outputs.remove(*key);
            let got = Record::decode(&cut);
            if skippable.contains(key) {
                assert!(got.is_ok(), "{kind} without {key}");
            } else {
                assert!(
                    matches!(&got, Err(CodecError::Missing { key: k, .. }) if k == key),
                    "{kind} without {key}: {got:?}"
                );
                assert!(got.expect_err("missing").to_string().contains(key));
            }
        }
    }
}

#[test]
fn a_value_that_is_not_in_the_form_the_writer_writes_is_refused() {
    let broken = |record: Record, key: &str, value: &str| {
        let mut written = record.encode();
        written.outputs.insert(key.to_owned(), value.to_owned());
        Record::decode(&written)
    };
    for number in [
        "",
        "+1",
        "01",
        " 1",
        "-1",
        "1.0",
        "18446744073709551616",
        "x",
    ] {
        for (record, key) in [(run(), "build"), (run(), "timestamp"), (seal(), "build")] {
            let got = broken(record, key, number);
            assert!(
                matches!(got, Err(CodecError::Bad { .. })),
                "{key} = {number:?}: {got:?}"
            );
        }
    }
    // A counter that is a number but too big for the width the record keeps.
    for (record, key) in [
        (stage(true), "attempt"),
        (validation(true), "k"),
        (publication(true), "attempt"),
    ] {
        let got = broken(record, key, "4294967296");
        assert!(matches!(got, Err(CodecError::Bad { .. })), "{key}: {got:?}");
    }
    for text in ["", "ABC", &"A".repeat(64), &"a".repeat(63)] {
        for (record, key) in [
            (run(), "machine"),
            (step(true), "input"),
            (stage(true), "set"),
            (seal(), "inputs"),
        ] {
            let got = broken(record, key, text);
            assert!(
                matches!(got, Err(CodecError::Bad { .. })),
                "{key} = {text:?}: {got:?}"
            );
        }
    }
    for flag in ["0", "2", "true", ""] {
        for (record, key) in [
            (stage(true), "confirmed"),
            (publication(true), "exceptional"),
        ] {
            let got = broken(record, key, flag);
            assert!(
                matches!(got, Err(CodecError::Bad { .. })),
                "{key} = {flag:?}: {got:?}"
            );
        }
    }
    let err = broken(run(), "build", "01").expect_err("bad");
    assert!(
        err.to_string().contains("build") && err.to_string().contains("01"),
        "{err}"
    );
}

#[test]
fn the_locator_is_where_the_kind_has_one_and_nowhere_else() {
    for record in every_record() {
        let written = record.encode();
        let mut flipped = written.clone();
        flipped.physical = match written.physical {
            Some(_) => None,
            None => Some(Locator::new("x")),
        };
        let err = Record::decode(&flipped).expect_err("the wrong locator");
        assert!(matches!(err, CodecError::Locator { .. }), "{err:?}");
        assert!(err.to_string().contains("locator"));
    }
}

#[test]
fn a_name_that_does_not_split_into_its_parts_is_refused() {
    let with_name = |record: Record, name: &str| {
        let mut written = record.encode();
        written.name = name.to_owned();
        Record::decode(&written)
    };
    for name in ["build", "/compile", "build/", ""] {
        for record in [step(true), validation(true), created()] {
            let got = with_name(record, name);
            assert!(
                matches!(got, Err(CodecError::Bad { .. })),
                "{name:?}: {got:?}"
            );
        }
    }
    for name in [
        "build",
        "build/linux",
        "build//rev",
        "/linux/rev",
        "build/linux/",
    ] {
        let got = with_name(seal(), name);
        assert!(
            matches!(got, Err(CodecError::Bad { .. })),
            "{name:?}: {got:?}"
        );
    }
    // A created object's id may hold a `/`: only the first splits the kind off.
    let mut slashed = created().encode();
    slashed.name = "kind/a/b".to_owned();
    slashed.physical = Some(Locator::new("a/b"));
    let got = Record::decode(&slashed).expect("ok");
    assert!(matches!(&got, Record::Created(c) if c.kind == "kind" && c.id == "a/b"));
}

#[test]
fn a_created_record_whose_name_and_locator_name_different_objects_is_refused() {
    let mut written = created().encode();
    written.physical = Some(Locator::new("another-object"));
    let got = Record::decode(&written);
    assert!(matches!(got, Err(CodecError::Bad { .. })), "{got:?}");
}
