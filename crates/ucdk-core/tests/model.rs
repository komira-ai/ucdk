// SPDX-License-Identifier: Apache-2.0
//! The shared model: record paths, keys, tags, locators, the clock and the format constants.

use std::time::Duration;

use ucdk_core::{
    Clock, Locator, ManualClock, ModelError, NODE_KEY_LEN, OwnerLabel, RecordKey, RecordPath,
    SEGMENT_MAX, Tag, millis,
};

#[test]
fn a_path_is_one_or_more_valid_segments() {
    for ok in [
        "a",
        "app/web",
        "a-1/b2/c-d",
        &"a".repeat(32),
        "a/b/c/d/e/f/g/h/i/j/k/l/m",
    ] {
        assert_eq!(RecordPath::new(ok).unwrap().as_str(), ok);
    }
    assert_eq!(RecordPath::new(""), Err(ModelError::EmptyPath));
    for bad in ["A", "1a", "a b", "a//b", "a/", "/a", "a_b", &"a".repeat(33)] {
        assert!(
            matches!(RecordPath::new(bad), Err(ModelError::BadSegment(_))),
            "{bad}"
        );
    }
    assert_eq!(
        RecordPath::new("app/Web"),
        Err(ModelError::BadSegment("Web".to_owned()))
    );
}

#[test]
fn the_owner_is_the_first_segment() {
    let path = RecordPath::new("app/web/leaf").unwrap();
    assert_eq!(path.owner().as_str(), "app");
    assert_eq!(path.owner(), OwnerLabel::new("app").unwrap());
    assert!(OwnerLabel::new("App").is_err());
    assert_eq!(path.to_string(), "app/web/leaf");
    assert_eq!(path.owner().to_string(), "app");
}

#[test]
fn a_node_key_is_26_base32_characters_that_depend_on_the_whole_path() {
    let key = RecordPath::new("app/web").unwrap().key();
    assert_eq!(key.as_str().len(), 26);
    assert!(
        key.as_str()
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    );
    assert_eq!(key, RecordKey::of(&RecordPath::new("app/web").unwrap()));
    for other in ["app/web2", "app/we", "web/app", "app"] {
        assert_ne!(key, RecordPath::new(other).unwrap().key(), "{other}");
    }
    assert_eq!(key.to_string(), key.as_str());
}

#[test]
fn a_node_key_is_pinned_to_its_scheme() {
    // The scheme is a promise to every ledger already written: this value must not change.
    assert_eq!(
        RecordPath::new("app/web").unwrap().key().as_str(),
        "5sxo3w6l42g34qiap3fg3dioqu"
    );
}

#[test]
fn a_key_is_read_back_only_in_its_own_shape() {
    let key = RecordPath::new("app/web").unwrap().key();
    assert_eq!(RecordKey::from_text(key.as_str()), Some(key));
    for bad in [
        "",
        "short",
        &"a".repeat(27),
        &"A".repeat(26),
        &"1".repeat(26),
    ] {
        assert_eq!(RecordKey::from_text(bad), None, "{bad}");
    }
}

#[test]
fn identifiers_print_as_they_were_given() {
    assert_eq!(Locator::new("p-1").to_string(), "p-1");
    assert_eq!(Locator::new("p-1").as_str(), "p-1");
    assert_eq!(Tag::new("x").as_str(), "x");
    assert_eq!(Tag::new("a kind").to_string(), "a kind");
    assert_eq!(Tag::new(""), Tag::new(String::new()), "any text is a tag");
}

#[test]
fn a_name_key_is_26_characters_that_depend_on_the_whole_name() {
    let key = RecordKey::of_name("pkg/web:1.0.0");
    assert_eq!(key.as_str().len(), NODE_KEY_LEN);
    assert!(
        key.as_str()
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    );
    assert_eq!(key, RecordKey::of_name("pkg/web:1.0.0"));
    for other in [
        "pkg/web:1.0.1",
        "pkg/web:1.0",
        "pkg/web",
        "web/pkg:1.0.0",
        "",
    ] {
        assert_ne!(key, RecordKey::of_name(other), "{other}");
    }
}

#[test]
fn a_name_key_differs_from_the_path_key_of_the_same_text() {
    for text in ["app/web", "app", "pkg/web"] {
        assert_ne!(
            RecordKey::of_name(text),
            RecordPath::new(text).unwrap().key(),
            "{text}"
        );
    }
}

#[test]
fn a_name_key_is_pinned_to_its_scheme() {
    // The golden text of a key in the domain `ucdk/record-key/1`: it must not change, and it is
    // the first 128 bits of SHA-256 over the domain, a newline and the name.
    assert_eq!(
        RecordKey::of_name("pkg/web:1.0.0").as_str(),
        "gcs3sk6hbogkhzbfp5qbd2y2v4"
    );
    assert_eq!(
        RecordKey::of_name("app/web").as_str(),
        "fsjmbdwsj22w5i5nwuzfgb5nte"
    );
}

#[test]
fn a_key_segment_is_k_then_the_key() {
    let key = RecordKey::of_name("pkg/web:1.0.0");
    assert_eq!(key.segment(), "kgcs3sk6hbogkhzbfp5qbd2y2v4");
    assert_eq!(key.segment().len(), 27);
    assert!(key.segment().len() <= SEGMENT_MAX);
}

#[test]
fn every_key_alphabet_character_forms_a_valid_segment() {
    let alphabet = "abcdefghijklmnopqrstuvwxyz234567";
    assert_eq!(alphabet.len(), 32);
    for first in alphabet.chars() {
        let text = format!("{first}{}", "a".repeat(NODE_KEY_LEN - 1));
        let key = RecordKey::from_text(&text).expect("a key of the alphabet");
        let path = RecordPath::new(&format!("tag/{}", key.segment()));
        assert!(path.is_ok(), "a key starting with {first}: {path:?}");
    }
}

#[test]
fn model_errors_read_as_sentences() {
    let model: Vec<String> = [ModelError::EmptyPath, ModelError::BadSegment("X".into())]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        model,
        ["a record path is empty", "`X` is not a valid path segment"]
    );
}

#[test]
fn the_format_constants_are_pinned() {
    // These values reach the ledger file: changing one is a format break for every ledger
    // already written.
    assert_eq!(SEGMENT_MAX, 32);
    assert_eq!(NODE_KEY_LEN, 26);
    assert!(RecordPath::new(&"a".repeat(SEGMENT_MAX)).is_ok());
    assert!(RecordPath::new(&"a".repeat(SEGMENT_MAX + 1)).is_err());
    assert!(OwnerLabel::new(&"a".repeat(SEGMENT_MAX)).is_ok());
    assert!(OwnerLabel::new(&"a".repeat(SEGMENT_MAX + 1)).is_err());
    let path = RecordPath::new("app/web").unwrap();
    assert_eq!(path.key().as_str().len(), NODE_KEY_LEN);
}

#[test]
fn a_manual_clock_moves_only_when_told_and_a_clone_shares_it() {
    let clock = ManualClock::at(1_000);
    let twin = clock.clone();
    assert_eq!(clock.now_millis(), 1_000);
    assert_eq!(clock.now_millis(), 1_000, "reading does not move it");
    twin.advance(Duration::from_millis(250));
    assert_eq!(clock.now_millis(), 1_250, "a clone shares the time");
    clock.advance(Duration::from_secs(2));
    assert_eq!(twin.now_millis(), 3_250);
    assert_eq!(ManualClock::default().now_millis(), 0);
}

#[test]
fn a_duration_in_milliseconds_saturates() {
    assert_eq!(millis(Duration::from_millis(1_500)), 1_500);
    assert_eq!(millis(Duration::MAX), u64::MAX);
}
