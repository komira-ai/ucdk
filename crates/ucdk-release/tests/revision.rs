// SPDX-License-Identifier: Apache-2.0
//! A revision is a full commit id, and a build stamp holds only checked values.

use ucdk_release::revision::{BuildStamp, Revision, RevisionError, StampError};

const REV: &str = "0123456789abcdef0123456789abcdef01234567";
const SRC: &str = "fedcba9876543210fedcba9876543210fedcba98";

const WHY: &str =
    "is not a full commit id (exactly 40 lowercase hex digits; an abbreviated id is refused)";

/// Text that is not a full commit id, each with what is wrong with it.
fn not_full_ids() -> Vec<(&'static str, String)> {
    vec![
        ("empty", String::new()),
        ("abbreviated", "a1b2c3d".to_owned()),
        ("39 digits", REV[..39].to_owned()),
        ("41 digits", format!("{REV}0")),
        ("upper case", REV.to_uppercase()),
        ("one upper-case digit", format!("A{}", &REV[1..])),
        ("not hex", format!("g{}", &REV[1..])),
        ("leading space", format!(" {}", &REV[1..])),
        ("trailing newline", format!("{}\n", &REV[..39])),
        ("trailing newline after 40", format!("{REV}\n")),
        ("prefixed", format!("0x{}", &REV[2..])),
        ("non-ascii of 40 bytes", format!("é{}", &REV[2..])),
        ("non-ascii digit of 40 chars", format!("{}é", &REV[..39])),
    ]
}

#[test]
fn a_full_lowercase_id_is_a_revision_and_reads_back_as_written() {
    let rev = Revision::parse(REV).expect("a revision");
    assert_eq!(rev.as_str(), REV);
    assert_eq!(rev.to_string(), REV);
    // Every digit of the alphabet is accepted, at both ends of each range.
    let all = "0123456789abcdef".repeat(3);
    assert_eq!(
        Revision::parse(&all[..40]).expect("a revision").as_str(),
        &all[..40]
    );
    assert_eq!(
        Revision::parse(&"0".repeat(40)).expect("zeros").as_str(),
        "0".repeat(40)
    );
    assert_eq!(
        Revision::parse(&"9".repeat(40)).expect("nines").as_str(),
        "9".repeat(40)
    );
    assert_eq!(
        Revision::parse(&"a".repeat(40)).expect("a").as_str(),
        "a".repeat(40)
    );
    assert_eq!(
        Revision::parse(&"f".repeat(40)).expect("f").as_str(),
        "f".repeat(40)
    );
}

#[test]
fn text_that_is_not_a_full_lowercase_id_is_refused_and_named() {
    for (what, text) in not_full_ids() {
        let err = Revision::parse(&text).expect_err(what);
        assert_eq!(err, RevisionError { text: text.clone() }, "{what}");
        assert_eq!(err.to_string(), format!("'{text}' {WHY}"), "{what}");
    }
}

#[test]
fn digits_just_outside_the_hex_alphabet_are_refused() {
    // The characters next to each end of '0'-'9' and 'a'-'f' in the ASCII table.
    for bad in ['/', ':', '`', 'g', '@', 'G', 'F', 'A'] {
        let text = format!("{bad}{}", &REV[1..]);
        assert!(Revision::parse(&text).is_err(), "{bad:?}");
    }
}

#[test]
fn revisions_compare_by_their_text() {
    let a = Revision::parse(REV).expect("a revision");
    assert_eq!(a, Revision::parse(REV).expect("a revision"));
    assert_ne!(a, Revision::parse(SRC).expect("a revision"));
    assert!(a < Revision::parse(SRC).expect("a revision"));
}

#[test]
fn a_stamp_with_full_ids_and_positive_numbers_is_accepted_and_keeps_every_value() {
    let stamp = BuildStamp::new(REV, SRC, 154, 1_790_994_309_000).expect("a stamp");
    assert_eq!(stamp.revision().as_str(), REV);
    assert_eq!(stamp.source_commit().as_str(), SRC);
    assert_eq!(stamp.build_number(), 154);
    assert_eq!(stamp.timestamp_ms(), 1_790_994_309_000);
}

#[test]
fn the_smallest_positive_numbers_are_accepted() {
    let stamp = BuildStamp::new(REV, REV, 1, 1).expect("a stamp");
    assert_eq!(stamp.build_number(), 1);
    assert_eq!(stamp.timestamp_ms(), 1);
    assert_eq!(stamp.revision(), stamp.source_commit());
}

#[test]
fn the_largest_numbers_are_accepted() {
    let stamp = BuildStamp::new(REV, SRC, i64::MAX, i64::MAX).expect("a stamp");
    assert_eq!(stamp.build_number(), i64::MAX as u64);
    assert_eq!(stamp.timestamp_ms(), i64::MAX as u64);
}

#[test]
fn a_revision_id_that_is_not_full_is_refused_by_field() {
    for (what, text) in not_full_ids() {
        let err = BuildStamp::new(&text, SRC, 1, 1).expect_err(what);
        assert_eq!(
            err,
            StampError::Revision {
                field: "revision_id",
                error: RevisionError { text: text.clone() },
            },
            "{what}"
        );
        assert_eq!(
            err.to_string(),
            format!("revision_id '{text}' {WHY}"),
            "{what}"
        );
    }
}

#[test]
fn a_source_commit_that_is_not_full_is_refused_by_field() {
    for (what, text) in not_full_ids() {
        let err = BuildStamp::new(REV, &text, 1, 1).expect_err(what);
        assert_eq!(
            err,
            StampError::Revision {
                field: "source_commit",
                error: RevisionError { text: text.clone() },
            },
            "{what}"
        );
        assert_eq!(
            err.to_string(),
            format!("source_commit '{text}' {WHY}"),
            "{what}"
        );
    }
}

#[test]
fn a_build_number_that_is_not_positive_is_refused() {
    for n in [0, -1, -3, i64::MIN] {
        let err = BuildStamp::new(REV, SRC, n, 1).expect_err("refused");
        assert_eq!(err, StampError::BuildNumber(n));
        assert_eq!(err.to_string(), format!("build_number {n} is not positive"));
    }
}

#[test]
fn a_timestamp_that_is_not_positive_is_refused() {
    for n in [0, -1, -3, i64::MIN] {
        let err = BuildStamp::new(REV, SRC, 1, n).expect_err("refused");
        assert_eq!(err, StampError::TimestampMs(n));
        assert_eq!(err.to_string(), format!("timestamp_ms {n} is not positive"));
    }
}

#[test]
fn the_first_failing_field_in_field_order_is_the_one_named() {
    let bad = "abc";
    // revision_id before source_commit before build_number before timestamp_ms.
    assert!(matches!(
        BuildStamp::new(bad, bad, 0, 0),
        Err(StampError::Revision {
            field: "revision_id",
            ..
        })
    ));
    assert!(matches!(
        BuildStamp::new(REV, bad, 0, 0),
        Err(StampError::Revision {
            field: "source_commit",
            ..
        })
    ));
    assert_eq!(
        BuildStamp::new(REV, SRC, 0, 0),
        Err(StampError::BuildNumber(0))
    );
    assert_eq!(
        BuildStamp::new(REV, SRC, 1, 0),
        Err(StampError::TimestampMs(0))
    );
}

#[test]
fn stamps_compare_by_every_field() {
    let base = BuildStamp::new(REV, SRC, 2, 3).expect("a stamp");
    assert_eq!(base, BuildStamp::new(REV, SRC, 2, 3).expect("a stamp"));
    for other in [
        BuildStamp::new(SRC, SRC, 2, 3),
        BuildStamp::new(REV, REV, 2, 3),
        BuildStamp::new(REV, SRC, 3, 3),
        BuildStamp::new(REV, SRC, 2, 4),
    ] {
        assert_ne!(base, other.expect("a stamp"));
    }
}

#[test]
fn a_refusal_message_holds_no_raw_newline_or_control_character_of_the_input() {
    // Text bound for a log or a CI summary: a newline could start a line of its own, and an
    // escape byte could restyle the lines after it.
    let text = "abc\n::error::x\u{1b}[31m\r\0\u{7f}";
    let from_revision = Revision::parse(text).expect_err("refused").to_string();
    let from_id = BuildStamp::new(text, SRC, 1, 1)
        .expect_err("refused")
        .to_string();
    let from_source = BuildStamp::new(REV, text, 1, 1)
        .expect_err("refused")
        .to_string();
    for message in [from_revision, from_id, from_source] {
        assert!(message.contains("::error::x"), "{message:?}");
        assert!(
            !message.chars().any(char::is_control),
            "a control character in {message:?}"
        );
    }
}
