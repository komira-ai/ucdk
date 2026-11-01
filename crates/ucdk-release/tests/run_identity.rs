// SPDX-License-Identifier: Apache-2.0
//! The grammar of the run id, the attempt and the context a driver supplies, and what makes two
//! identities the same run.

use std::collections::BTreeMap;

use ucdk_release::run_identity::{
    CONTEXT_KEY_MAX_BYTES, CONTEXT_MAX_ENTRIES, CONTEXT_VALUE_MAX_BYTES, RUN_ID_MAX_BYTES,
    RunIdentityError as E, check, from_flags, parse_attempt, same_as,
};
use ucdk_release_proto::v1::RunIdentity;

fn identity(run_id: &str, attempt: u32, context: &[(&str, &str)]) -> RunIdentity {
    RunIdentity {
        run_id: run_id.to_owned(),
        attempt,
        context: context
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
    }
}

fn flags(run_id: &str, attempt: &str, context: &[&str]) -> Result<RunIdentity, E> {
    let context: Vec<String> = context.iter().map(|c| (*c).to_owned()).collect();
    from_flags(run_id, Some(attempt), &context)
}

#[test]
fn flags_build_the_identity_they_name() {
    let built = flags("gh-123_a", "2", &["actor=ci", "branch=main"]).expect("valid");
    assert_eq!(
        built,
        identity("gh-123_a", 2, &[("actor", "ci"), ("branch", "main")])
    );
    assert_eq!(check(&built), Ok(()));
}

#[test]
fn equal_inputs_are_the_same_run() {
    let a = identity("run-1", 1, &[("actor", "ci")]);
    let b = identity("run-1", 1, &[("actor", "ci")]);
    assert!(same_as(&a, &b));
    assert!(same_as(&b, &a));
    let from_flags_a = flags("run-1", "1", &["actor=ci"]).expect("valid");
    assert!(same_as(&a, &from_flags_a));
}

#[test]
fn any_changed_input_is_another_run() {
    let base = identity("run-1", 1, &[("actor", "ci")]);
    let others = [
        identity("run-2", 1, &[("actor", "ci")]),
        identity("run-1", 2, &[("actor", "ci")]),
        identity("run-1", 1, &[("actor", "bot")]),
        identity("run-1", 1, &[("agent", "ci")]),
        identity("run-1", 1, &[("actor", "ci"), ("branch", "main")]),
        identity("run-1", 1, &[]),
    ];
    for other in &others {
        assert!(!same_as(&base, other), "{other:?}");
        assert!(!same_as(other, &base), "{other:?}");
    }
}

#[test]
fn the_run_id_is_one_to_sixty_three_label_safe_bytes() {
    let longest = "a".repeat(RUN_ID_MAX_BYTES);
    assert_eq!(check(&identity(&longest, 1, &[])), Ok(()));
    assert_eq!(check(&identity("a-b_c-0", 1, &[])), Ok(()));
    assert_eq!(check(&identity("", 1, &[])), Err(E::RunIdEmpty));
    let too_long = "a".repeat(RUN_ID_MAX_BYTES + 1);
    assert_eq!(
        check(&identity(&too_long, 1, &[])),
        Err(E::RunIdTooLong {
            bytes: RUN_ID_MAX_BYTES + 1
        })
    );
    for (text, at) in [
        ("Run", 0),
        ("run 1", 3),
        ("run.1", 3),
        ("run/1", 3),
        ("ru\u{e9}", 2),
        ("run\n", 3),
    ] {
        assert_eq!(
            check(&identity(text, 1, &[])),
            Err(E::RunIdByte { at }),
            "{text:?}"
        );
    }
}

#[test]
fn the_attempt_has_no_default() {
    assert_eq!(check(&identity("run-1", 0, &[])), Err(E::AttemptMissing));
    assert_eq!(
        from_flags("run-1", None, &[]),
        Err(E::AttemptMissing),
        "an omitted attempt is a usage error, never attempt 1"
    );
}

#[test]
fn an_attempt_is_a_positive_decimal_number() {
    assert_eq!(parse_attempt("1"), Ok(1));
    assert_eq!(parse_attempt("17"), Ok(17));
    assert_eq!(parse_attempt("4294967295"), Ok(u32::MAX));
    for text in [
        "",
        "0",
        "007",
        "01",
        "+1",
        "-1",
        " 1",
        "1 ",
        "1.5",
        "abc",
        "\u{663}",
        "4294967296",
    ] {
        assert_eq!(
            parse_attempt(text),
            Err(E::AttemptText {
                text: text.to_owned()
            }),
            "{text:?}"
        );
    }
    assert_eq!(
        flags("run-1", "0", &[]),
        Err(E::AttemptText {
            text: "0".to_owned()
        })
    );
}

#[test]
fn a_context_key_is_a_lowercase_word_of_at_most_thirty_two_bytes() {
    let longest = format!("k{}", "0".repeat(CONTEXT_KEY_MAX_BYTES - 1));
    assert_eq!(check(&identity("r", 1, &[(longest.as_str(), "v")])), Ok(()));
    assert_eq!(check(&identity("r", 1, &[("a_1", "v")])), Ok(()));
    let too_long = format!("k{}", "0".repeat(CONTEXT_KEY_MAX_BYTES));
    for key in [
        "",
        "A",
        "1a",
        "_a",
        "a-b",
        "a b",
        "a.b",
        "\u{e9}",
        too_long.as_str(),
    ] {
        assert_eq!(
            check(&identity("r", 1, &[(key, "v")])),
            Err(E::ContextKey {
                key: key.to_owned()
            }),
            "{key:?}"
        );
    }
}

#[test]
fn a_context_value_is_at_most_256_bytes_of_printable_ascii() {
    let longest = "x".repeat(CONTEXT_VALUE_MAX_BYTES);
    assert_eq!(check(&identity("r", 1, &[("k", longest.as_str())])), Ok(()));
    assert_eq!(check(&identity("r", 1, &[("k", "")])), Ok(()));
    assert_eq!(check(&identity("r", 1, &[("k", " ~ ")])), Ok(()));
    let too_long = "x".repeat(CONTEXT_VALUE_MAX_BYTES + 1);
    assert_eq!(
        check(&identity("r", 1, &[("k", too_long.as_str())])),
        Err(E::ContextValueTooLong {
            key: "k".to_owned(),
            bytes: CONTEXT_VALUE_MAX_BYTES + 1
        })
    );
    for (value, at) in [
        ("a\tb", 1),
        ("a\nb", 1),
        ("ab\u{1f}", 2),
        ("\u{7f}", 0),
        ("ab\u{e9}", 2),
    ] {
        assert_eq!(
            check(&identity("r", 1, &[("k", value)])),
            Err(E::ContextValueByte {
                key: "k".to_owned(),
                at
            }),
            "{value:?}"
        );
    }
}

#[test]
fn at_most_thirty_two_context_pairs_are_kept() {
    let keys: Vec<String> = (0..=CONTEXT_MAX_ENTRIES).map(|n| format!("k{n}")).collect();
    let pairs: Vec<(&str, &str)> = keys.iter().map(|k| (k.as_str(), "v")).collect();
    assert_eq!(
        check(&identity("r", 1, &pairs[..CONTEXT_MAX_ENTRIES])),
        Ok(())
    );
    assert_eq!(
        check(&identity("r", 1, &pairs)),
        Err(E::ContextTooMany {
            count: CONTEXT_MAX_ENTRIES + 1
        })
    );
    let args: Vec<String> = keys.iter().map(|k| format!("{k}=v")).collect();
    assert_eq!(
        from_flags("r", Some("1"), &args),
        Err(E::ContextTooMany {
            count: CONTEXT_MAX_ENTRIES + 1
        })
    );
    assert!(from_flags("r", Some("1"), &args[..CONTEXT_MAX_ENTRIES]).is_ok());
}

#[test]
fn a_context_argument_is_split_at_its_first_equals_sign() {
    let built = flags("r", "1", &["url=https://example.com/?a=b", "empty="]).expect("valid");
    assert_eq!(
        built.context.get("url").map(String::as_str),
        Some("https://example.com/?a=b")
    );
    assert_eq!(built.context.get("empty").map(String::as_str), Some(""));
    assert_eq!(
        flags("r", "1", &["no-equals"]),
        Err(E::ContextNotKeyValue {
            arg: "no-equals".to_owned()
        })
    );
    assert_eq!(
        flags("r", "1", &["Bad=v"]),
        Err(E::ContextKey {
            key: "Bad".to_owned()
        })
    );
    assert_eq!(
        flags("r", "1", &["k=\u{1f}"]),
        Err(E::ContextValueByte {
            key: "k".to_owned(),
            at: 0
        })
    );
}

#[test]
fn a_context_key_given_twice_is_refused_not_overwritten() {
    assert_eq!(
        flags("r", "1", &["k=a", "k=b"]),
        Err(E::ContextKeyTwice {
            key: "k".to_owned()
        })
    );
}

#[test]
fn the_first_fault_is_the_run_id_then_the_attempt_then_the_context() {
    assert_eq!(flags("", "0", &["Bad=v"]), Err(E::RunIdEmpty));
    assert_eq!(
        flags("r", "0", &["Bad=v"]),
        Err(E::AttemptText {
            text: "0".to_owned()
        })
    );
    assert_eq!(
        flags("r", "1", &["Bad=v"]),
        Err(E::ContextKey {
            key: "Bad".to_owned()
        })
    );
}

#[test]
fn every_refusal_reads_as_a_sentence_that_names_what_it_refused() {
    let k = |s: &str| s.to_owned();
    let cases = [
        (E::RunIdEmpty, "empty"),
        (E::RunIdTooLong { bytes: 70 }, "70 bytes"),
        (E::RunIdByte { at: 3 }, "at 3"),
        (E::AttemptMissing, "no default"),
        (E::AttemptText { text: k("0x") }, "'0x'"),
        (E::ContextTooMany { count: 40 }, "40 context"),
        (E::ContextNotKeyValue { arg: k("abc") }, "'abc'"),
        (E::ContextKey { key: k("Bad") }, "'Bad'"),
        (E::ContextKeyTwice { key: k("dup") }, "'dup'"),
        (
            E::ContextValueTooLong {
                key: k("long"),
                bytes: 300,
            },
            "300 bytes",
        ),
        (
            E::ContextValueByte {
                key: k("odd"),
                at: 5,
            },
            "at 5",
        ),
    ];
    for (error, want) in cases {
        let text = error.to_string();
        assert!(text.contains(want), "{text:?} lacks {want:?}");
        assert!(!text.contains('\n'), "{text:?}");
    }
    let boxed: Box<dyn std::error::Error> = Box::new(E::RunIdEmpty);
    assert_eq!(boxed.to_string(), E::RunIdEmpty.to_string());
}
