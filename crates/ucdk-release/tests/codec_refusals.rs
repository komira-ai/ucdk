// SPDX-License-Identifier: Apache-2.0
//! What the record codec refuses: a payload that disagrees with itself, a name that does not
//! split into its parts, a part that is not a part of a store key, a value that is not in the
//! form the writer writes.

mod codec_records;

use codec_records::{
    action, action_of, every_record, grant, grant_of, publication, publication_of, run, run_of,
    seal, seal_of, stage, stage_of, validation, validation_of, written,
};
use ucdk_core::{Locator, RecordKey, Tag};
use ucdk_release::codec::{
    Action, CodecError, Encoded, Grant, Phase, Publication, Record, Run, Seal, Stage, Validation,
    keys_of,
};
use ucdk_release::transitions::Kind;

#[test]
fn a_grant_that_disagrees_with_itself_is_refused() {
    let bad = |edit: &dyn Fn(&mut Encoded)| {
        let mut encoded = written(&grant());
        edit(&mut encoded);
        Record::decode(&encoded)
    };
    let bad_key = |got: &Result<Record, CodecError>, wanted: &str| {
        assert!(
            matches!(got, Err(CodecError::Bad { key, .. }) if key == wanted),
            "{wanted}: {got:?}"
        );
    };
    // The phase of the name and the phase of the payload are one.
    bad_key(
        &bad(&|e| e.name = "publish/push/commit/a2-1".to_owned()),
        "name",
    );
    // A word that is not a phase, an outcome or a fault.
    for (key, value) in [("phase", "write"), ("outcome", "pending"), ("outcome", "")] {
        let got = bad(&|e| {
            e.outputs.insert(key.to_owned(), value.to_owned());
        });
        assert!(
            matches!(&got, Err(CodecError::Bad { key: k, value: v, .. }) if k == key && v == value),
            "{key}={value}: {got:?}"
        );
    }
    bad_key(
        &bad(&|e| {
            e.outputs.insert("outcome".to_owned(), "refused".to_owned());
            e.outputs.remove("expires_ms");
            e.outputs.insert("fault".to_owned(), "gone".to_owned());
        }),
        "fault",
    );
    // Granted needs an expiry and carries no fault; refused needs a fault and carries no
    // expiry; an unanswered exchange carries neither.
    let got = bad(&|e| {
        e.outputs.remove("expires_ms");
    });
    assert!(
        matches!(
            &got,
            Err(CodecError::Missing {
                key: "expires_ms",
                ..
            })
        ),
        "{got:?}"
    );
    bad_key(
        &bad(&|e| {
            e.outputs.insert("fault".to_owned(), "denied".to_owned());
        }),
        "fault",
    );
    bad_key(
        &bad(&|e| {
            e.outputs.insert("outcome".to_owned(), "refused".to_owned());
            e.outputs.insert("fault".to_owned(), "denied".to_owned());
        }),
        "expires_ms",
    );
    let got = bad(&|e| {
        e.outputs.insert("outcome".to_owned(), "refused".to_owned());
        e.outputs.remove("expires_ms");
    });
    assert!(
        matches!(&got, Err(CodecError::Missing { key: "fault", .. })),
        "{got:?}"
    );
    bad_key(
        &bad(&|e| {
            e.outputs.remove("outcome");
        }),
        "expires_ms",
    );
    bad_key(
        &bad(&|e| {
            e.outputs.remove("outcome");
            e.outputs.remove("expires_ms");
            e.outputs.insert("fault".to_owned(), "denied".to_owned());
        }),
        "fault",
    );
    bad_key(
        &bad(&|e| {
            e.outputs.insert("expires_ms".to_owned(), "soon".to_owned());
        }),
        "expires_ms",
    );
}

#[test]
fn a_grant_name_is_stage_holder_phase_and_the_exchange() {
    let with_name = |name: &str| {
        let mut encoded = written(&grant());
        encoded.name = name.to_owned();
        Record::decode(&encoded)
    };
    assert!(with_name("publish/push/stage/a2-1").is_ok());
    assert!(with_name("publish/push/stage/a2-0").is_ok());
    for name in [
        // The wrong number of parts.
        "",
        "publish",
        "publish/push/stage",
        "publish/push/stage/a2-1/extra",
        // The exchange is `a<attempt>-<n>` in the form the writer writes.
        "publish/push/stage/2-1",
        "publish/push/stage/b2-1",
        "publish/push/stage/a2",
        "publish/push/stage/a-1",
        "publish/push/stage/a2-",
        "publish/push/stage/a02-1",
        "publish/push/stage/a2-01",
        "publish/push/stage/a+2-1",
        "publish/push/stage/a2--1",
        "publish/push/stage/a2-1x",
        // The counts fit the width the record keeps.
        "publish/push/stage/a4294967296-1",
        "publish/push/stage/a2-4294967296",
        // Attempts count from 1.
        "publish/push/stage/a0-1",
    ] {
        let got = with_name(name);
        assert!(
            matches!(got, Err(CodecError::Bad { .. })),
            "{name:?}: {got:?}"
        );
    }
    for (name, part) in [
        ("/push/stage/a2-1", "stage"),
        ("publish//stage/a2-1", "holder"),
        ("publish/../stage/a2-1", "holder"),
        ("publish/./stage/a2-1", "holder"),
        ("publish/push//a2-1", "phase"),
        ("publish/push/stage/", "exchange"),
    ] {
        let got = with_name(name);
        assert!(
            matches!(&got, Err(CodecError::Part { part: p, .. }) if *p == part),
            "{name:?}: {got:?}"
        );
    }
}

#[test]
fn the_kinds_have_no_locator() {
    for record in every_record() {
        assert_eq!(written(&record).physical, None);
        let mut with = written(&record);
        with.physical = Some(Locator::new("x"));
        let err = Record::decode(&with).expect_err("a locator");
        assert!(matches!(err, CodecError::Locator { .. }), "{err:?}");
        assert!(err.to_string().contains("locator"));
    }
    assert_eq!(written(&seal()).name, "build/linux-x86_64/0123456789abcdef");
}

#[test]
fn a_record_path_is_its_tag_and_the_key_of_its_name() {
    let encoded = written(&publication(true, None));
    let path = encoded.path().expect("a path");
    let key = RecordKey::of_name(&encoded.name);
    assert_eq!(path.as_str(), format!("publication/k{key}"));
    assert_eq!(path.owner().as_str(), "publication");
    // The name keeps the characters a path segment cannot, and the path is still valid; another
    // name is another path.
    let mut other = encoded.clone();
    other.name = "stable/file/tool/1.2.4/linux-x86_64".to_owned();
    assert_ne!(other.path().expect("path"), path);
    for record in every_record() {
        written(&record)
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
    // The kinds of the build-system model only: a step, a created object and a deploy are not
    // records of it.
    for tag in ["deploy", "step", "created", ""] {
        let mut encoded = written(&run());
        encoded.kind = Tag::new(tag);
        let err = Record::decode(&encoded).expect_err("unknown kind");
        assert_eq!(err, CodecError::UnknownKind(tag.to_owned()));
        assert!(err.to_string().contains(tag));
    }
}

#[test]
fn a_required_key_that_is_missing_is_refused() {
    // The keys a record of the kind may leave out. A grant's answer keys depend on each other
    // and are held by `a_grant_that_disagrees_with_itself_is_refused`.
    let optional: [(Kind, &[&str]); 5] = [
        (Kind::Stage, &["platform", "set", "generation", "confirmed"]),
        (Kind::Action, &["set", "generation"]),
        (Kind::Publication, &["exceptional", "phase"]),
        (Kind::Validation, &["set"]),
        (Kind::Grant, &["outcome", "fault", "expires_ms"]),
    ];
    for record in [
        run(),
        stage(true),
        action(true),
        publication(true, Some(Phase::Staging)),
        validation(true),
        grant(),
        seal(),
    ] {
        let encoded = written(&record);
        let kind = record.kind();
        let skippable: &[&str] = optional
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(&[], |(_, o)| *o);
        for key in keys_of(kind) {
            let mut cut = encoded.clone();
            if cut.outputs.remove(*key).is_none() {
                continue;
            }
            let got = Record::decode(&cut);
            if skippable.contains(key) {
                if kind != Kind::Grant {
                    assert!(got.is_ok(), "{kind:?} without {key}");
                }
            } else {
                assert!(
                    matches!(&got, Err(CodecError::Missing { key: k, .. }) if k == key),
                    "{kind:?} without {key}: {got:?}"
                );
                assert!(got.expect_err("missing").to_string().contains(key));
            }
        }
    }
}

#[test]
fn a_value_that_is_not_in_the_form_the_writer_writes_is_refused() {
    let broken = |record: Record, key: &str, value: &str| {
        let mut encoded = written(&record);
        encoded.outputs.insert(key.to_owned(), value.to_owned());
        Record::decode(&encoded)
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
        for (record, key) in [
            (run(), "build"),
            (run(), "timestamp"),
            (seal(), "build"),
            (grant(), "expires_ms"),
        ] {
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
        (publication(true, None), "attempt"),
    ] {
        let got = broken(record, key, "4294967296");
        assert!(matches!(got, Err(CodecError::Bad { .. })), "{key}: {got:?}");
    }
    for text in ["", "ABC", &"A".repeat(64), &"a".repeat(63)] {
        for (record, key) in [
            (run(), "machine"),
            (action(true), "input"),
            (stage(true), "set"),
            (seal(), "inputs"),
            (grant(), "coordinates"),
            (validation(true), "set"),
            (action(true), "set"),
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
            (publication(true, None), "exceptional"),
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
fn a_name_that_does_not_split_into_its_parts_is_refused() {
    let with_name = |record: Record, name: &str| {
        let mut encoded = written(&record);
        encoded.name = name.to_owned();
        Record::decode(&encoded)
    };
    for name in ["build", "build/compile/extra", ""] {
        for record in [action(true), validation(true)] {
            let got = with_name(record, name);
            assert!(
                matches!(got, Err(CodecError::Bad { ref key, .. }) if key == "name"),
                "{name:?}: {got:?}"
            );
        }
    }
    for name in ["build", "build/linux", "build/linux/rev/extra", ""] {
        let got = with_name(seal(), name);
        assert!(
            matches!(got, Err(CodecError::Bad { ref key, .. }) if key == "name"),
            "{name:?}: {got:?}"
        );
    }
    // A part that is empty is not a part.
    for name in ["/compile", "build/"] {
        for record in [action(true), validation(true)] {
            let got = with_name(record, name);
            assert!(
                matches!(got, Err(CodecError::Part { .. })),
                "{name:?}: {got:?}"
            );
        }
    }
    for name in ["build//rev", "/linux/rev", "build/linux/"] {
        let got = with_name(seal(), name);
        assert!(
            matches!(got, Err(CodecError::Part { .. })),
            "{name:?}: {got:?}"
        );
    }
}

/// A record of each kind with a part of a name, or a generation, set to `part`.
fn parts_records(part: &str) -> Vec<Record> {
    let p = || part.to_owned();
    let s = stage_of(stage(true));
    let a = action_of(action(true));
    let v = validation_of(validation(true));
    let sl = seal_of(seal());
    let g = grant_of(grant());
    let pb = publication_of(publication(true, None));
    let r = run_of(run());
    vec![
        Record::Run(Run {
            run_id: p(),
            ..r.clone()
        }),
        Record::Run(Run { revision: p(), ..r }),
        Record::Stage(Stage {
            stage: p(),
            ..s.clone()
        }),
        Record::Stage(Stage {
            platform: Some(p()),
            ..s.clone()
        }),
        Record::Stage(Stage {
            generation: Some(p()),
            ..s
        }),
        Record::Action(Action {
            stage: p(),
            ..a.clone()
        }),
        Record::Action(Action {
            action: p(),
            ..a.clone()
        }),
        Record::Action(Action {
            generation: Some(p()),
            ..a
        }),
        Record::Validation(Validation {
            stage: p(),
            ..v.clone()
        }),
        Record::Validation(Validation {
            validation: p(),
            ..v
        }),
        Record::Seal(Seal {
            stage: p(),
            ..sl.clone()
        }),
        Record::Seal(Seal {
            platform: p(),
            ..sl.clone()
        }),
        Record::Seal(Seal {
            revision: p(),
            ..sl.clone()
        }),
        Record::Seal(Seal {
            generation: p(),
            ..sl.clone()
        }),
        Record::Seal(Seal { run: p(), ..sl }),
        Record::Grant(Grant {
            stage: p(),
            ..g.clone()
        }),
        Record::Grant(Grant { holder: p(), ..g }),
        Record::Publication(Publication {
            stage: p(),
            ..pb.clone()
        }),
        Record::Publication(Publication { run: p(), ..pb }),
    ]
}

#[test]
fn parts_are_held_to_the_store_key_part_rule() {
    // The rule of `store::is_plain`: a part is not empty, is not `.` or `..`, and holds no `/`,
    // `\` or control character. A part read back from a record can then be put in a store key.
    let bad = [
        "", ".", "..", "a\\b", "a/b", "a\nb", "a\u{0}b", "a\u{7f}b", "a\tb",
    ];
    let good = [
        "a",
        "...",
        "a.b",
        "é",
        "Tool",
        "a b",
        "a:b",
        "1.2.3+build.4",
    ];
    for part in good {
        for record in parts_records(part) {
            let encoded = record.encode();
            assert!(encoded.is_ok(), "{part:?}: {encoded:?}");
            assert_eq!(Record::decode(&encoded.expect("ok")), Ok(record));
        }
    }
    for part in bad {
        for record in parts_records(part) {
            let err = record.encode().expect_err("a part that is not plain");
            assert!(
                matches!(err, CodecError::Part { .. }),
                "{part:?}: {record:?}: {err:?}"
            );
            assert!(err.to_string().contains("control character"));
        }
    }
    // The reader holds payload parts and name parts to the same rule, whatever wrote them.
    for part in bad {
        for (record, key) in [
            (run(), "revision"),
            (stage(true), "generation"),
            (stage(true), "platform"),
            (action(true), "generation"),
            (seal(), "generation"),
            (publication(true, None), "stage"),
            (publication(true, None), "run"),
            (seal(), "run"),
        ] {
            let mut encoded = written(&record);
            encoded.outputs.insert(key.to_owned(), part.to_owned());
            let got = Record::decode(&encoded);
            assert!(
                matches!(&got, Err(CodecError::Part { part: p, .. }) if *p == key),
                "{key} = {part:?}: {got:?}"
            );
        }
        let mut named_run = written(&run());
        named_run.name = part.to_owned();
        assert!(
            matches!(
                Record::decode(&named_run),
                Err(CodecError::Part { part: "run_id", .. })
            ),
            "{part:?}"
        );
        let mut named_stage = written(&stage(true));
        named_stage.name = part.to_owned();
        assert!(
            matches!(
                Record::decode(&named_stage),
                Err(CodecError::Part { part: "stage", .. })
            ),
            "{part:?}"
        );
    }
}

#[test]
fn an_empty_stage_platform_is_refused() {
    // No platform is the key absent. A platform that is present is a part of a store key, so
    // an empty one is refused by the writer and by the reader, and is never read as "none".
    let none = Stage {
        platform: Some(String::new()),
        ..stage_of(stage(true))
    };
    let err = Record::Stage(none).encode().expect_err("an empty platform");
    assert!(
        matches!(
            err,
            CodecError::Part {
                part: "platform",
                ..
            }
        ),
        "{err:?}"
    );
    let mut encoded = written(&stage(true));
    encoded.outputs.insert("platform".to_owned(), String::new());
    let got = Record::decode(&encoded);
    assert!(
        matches!(
            &got,
            Err(CodecError::Part {
                part: "platform",
                ..
            })
        ),
        "{got:?}"
    );
}

#[test]
fn a_part_with_a_slash_cannot_make_two_records_meet() {
    // Without the rule, these two seals would have one name and so one path.
    let one = Seal {
        stage: "a/b".to_owned(),
        platform: "c".to_owned(),
        ..seal_of(seal())
    };
    let other = Seal {
        stage: "a".to_owned(),
        platform: "b/c".to_owned(),
        ..seal_of(seal())
    };
    assert!(Record::Seal(one).encode().is_err());
    assert!(Record::Seal(other).encode().is_err());
}
