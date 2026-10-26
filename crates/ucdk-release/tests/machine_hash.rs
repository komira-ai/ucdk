// SPDX-License-Identifier: Apache-2.0
//! The machine hash: the canonical encoding of the machine under its domain tag.

mod common;

use common::*;
use prost::Message;
use ucdk_core::Framed;
use ucdk_release::hashes::machine_hash;
use ucdk_release_proto::v1::step::Action;
use ucdk_release_proto::v1::{Machine, Subject};

#[test]
fn golden_text() {
    let small = Machine {
        schema_version: 1,
        name: "m".to_owned(),
        stamp_ignore: vec!["x".to_owned(), "y".to_owned()],
        ..Machine::default()
    };
    // The canonical bytes, written by hand: field 1 (varint) = 1, field 2 (text) = "m", and
    // field 6 (text) twice. Fields come in number order and the defaults are left out.
    let bytes = [
        0x08, 0x01, 0x12, 0x01, b'm', 0x32, 0x01, b'x', 0x32, 0x01, b'y',
    ];
    assert_eq!(small.encode_to_vec(), bytes);
    let by_hand = Framed::new(b"ucdk/release-machine/1\n")
        .field(&bytes)
        .finish();
    assert_eq!(machine_hash(&small), by_hand);
    assert_eq!(
        machine_hash(&small).as_str(),
        "a85eec5aa9ab2ce4e5f1c45643d09df5e8843a59bae3c95ed4141eb7a4d52dcf",
        "the hash of the small machine is a promise: a change here voids every approval"
    );
}

#[test]
fn map_entries_are_encoded_in_key_order() {
    let with_options = |keys: [&str; 2]| {
        let mut m = good();
        let v = &mut m.stages[1].steps[0].validations[0];
        for key in keys {
            v.options.insert(key.to_owned(), key.to_owned());
        }
        m
    };
    // The same keys inserted in the other order are the same machine.
    assert_eq!(
        machine_hash(&with_options(["a", "b"])),
        machine_hash(&with_options(["b", "a"]))
    );
    // And the bytes carry the entries in key order: field 6, each entry a key and a value.
    let mut v = validation("v", "fake", Subject::LocalSet);
    v.name.clear();
    v.validator.clear();
    v.subject = 0;
    v.rows.clear();
    v.options.insert("b".to_owned(), "2".to_owned());
    v.options.insert("a".to_owned(), "1".to_owned());
    assert_eq!(
        v.encode_to_vec(),
        [
            0x32, 6, 0x0A, 1, b'a', 0x12, 1, b'1', //
            0x32, 6, 0x0A, 1, b'b', 0x12, 1, b'2',
        ]
    );
}

#[test]
fn the_same_machine_hashes_the_same_and_any_edit_changes_it() {
    let base = machine_hash(&good());
    assert_eq!(machine_hash(&good()), base);
    let edits: Vec<Edit> = vec![
        Box::new(|m| m.name = "other".to_owned()),
        Box::new(|m| m.stamp_ignore.clear()),
        Box::new(|m| m.channels[0].monotonic = true),
        Box::new(|m| {
            m.artifacts[0]
                .builder
                .as_mut()
                .expect("builder")
                .args
                .clear();
        }),
        Box::new(|m| m.stages[1].steps[0].validations[0].subject = Subject::ChannelReadback as i32),
        Box::new(|m| {
            if let Some(Action::Publish(p)) = &mut m.stages[2].steps[0].action {
                p.attempts = 5;
            }
        }),
        Box::new(|m| m.stages[2].environment = "publish".to_owned()),
    ];
    let mut seen = vec![base.to_string()];
    for edit in &edits {
        let mut m = good();
        edit(&mut m);
        seen.push(machine_hash(&m).to_string());
    }
    let count = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen.len(),
        count,
        "an edit left the hash as it was, or two edits shared one"
    );
}
