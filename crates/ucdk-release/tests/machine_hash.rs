// SPDX-License-Identifier: Apache-2.0
//! The machine hash: the canonical encoding of the machine under its domain tag.

mod common;

use common::*;
use prost::Message;
use ucdk_core::Framed;
use ucdk_release::hashes::machine_hash;
use ucdk_release_proto::v1::{ArtifactType, BuildSystemKind, Machine, ValidationKind};

type Edit = fn(&mut Machine);

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
    let with_env = |keys: [&str; 2]| {
        let mut m = good();
        for key in keys {
            system_mut(&mut m)
                .env
                .insert(key.to_owned(), key.to_owned());
            artifact_mut(&mut m, "ucdk-cli")
                .target
                .as_mut()
                .expect("target")
                .env
                .insert(key.to_owned(), key.to_owned());
            artifact_mut(&mut m, "ucdk-cli")
                .labels
                .insert(key.to_owned(), key.to_owned());
        }
        m
    };
    // The same keys inserted in the other order are the same machine.
    assert_eq!(
        machine_hash(&with_env(["A", "B"])),
        machine_hash(&with_env(["B", "A"]))
    );
    // And the bytes carry the entries in key order: field 6 of the build system, each entry a
    // key and a value.
    let system = ucdk_release_proto::v1::BuildSystem {
        env: pairs(&[("B", "2"), ("A", "1")]),
        ..ucdk_release_proto::v1::BuildSystem::default()
    };
    assert_eq!(
        system.encode_to_vec(),
        [
            0x32, 6, 0x0A, 1, b'A', 0x12, 1, b'1', //
            0x32, 6, 0x0A, 1, b'B', 0x12, 1, b'2',
        ]
    );
}

#[test]
fn the_same_machine_hashes_the_same() {
    assert_eq!(machine_hash(&good()), machine_hash(&good()));
}

#[test]
fn covers_the_build_system_and_every_target() {
    // An approver approves every flag and variable a build or a test will run with, so each
    // field of the build system, of each artifact's target and of each test target changes
    // the hash, and no two edits share one.
    let edits: Vec<(&str, Edit)> = vec![
        ("kind", |m| {
            system_mut(m).kind = BuildSystemKind::Bazel as i32;
        }),
        ("binary", |m| {
            system_mut(m).binary = "/usr/bin/buck2".to_owned();
        }),
        ("config file", |m| system_mut(m).config_file.clear()),
        ("mode", |m| system_mut(m).mode = "debug".to_owned()),
        ("base flag", |m| {
            system_mut(m).flags.push("--keep-going".to_owned());
        }),
        ("base flag order", |m| {
            system_mut(m).flags = strings(&["-b", "-a"]);
        }),
        ("base env value", |m| {
            system_mut(m)
                .env
                .insert("PATH".to_owned(), "/bin".to_owned());
        }),
        ("base env key", |m| {
            system_mut(m).env.insert("HOME".to_owned(), "/h".to_owned());
        }),
        ("artifact target label", |m| {
            artifact_mut(m, "ucdk-lib")
                .target
                .as_mut()
                .expect("t")
                .label = "//release:other".to_owned();
        }),
        ("artifact target flag", |m| {
            artifact_mut(m, "ucdk-lib")
                .target
                .as_mut()
                .expect("t")
                .flags
                .pop();
        }),
        ("artifact target env", |m| {
            artifact_mut(m, "ucdk-lib")
                .target
                .as_mut()
                .expect("t")
                .env
                .insert("K".to_owned(), "V".to_owned());
        }),
        ("artifact labels", |m| {
            artifact_mut(m, "ucdk-lib")
                .labels
                .insert("requires".to_owned(), "x".to_owned());
        }),
        ("artifact type", |m| {
            artifact_mut(m, "ucdk-lib").r#type = ArtifactType::Oci as i32;
        }),
        ("test target label", |m| {
            test_mut(m, "build", "test").targets[0].label = "//release:other_test".to_owned();
        }),
        ("test target flag", |m| {
            test_mut(m, "build", "test").targets[0]
                .flags
                .push("--nocache".to_owned());
        }),
        ("test target env", |m| {
            test_mut(m, "build", "test").targets[0]
                .env
                .insert("K".to_owned(), "V".to_owned());
        }),
        ("placeholder as written", |m| {
            artifact_mut(m, "ucdk-cli")
                .target
                .as_mut()
                .expect("t")
                .flags[1] = "release.version={revision}".to_owned();
        }),
        ("name", |m| m.name = "other".to_owned()),
        ("stamp ignore", |m| {
            m.stamp_ignore.push("docs/**".to_owned());
        }),
        ("validation kind", |m| {
            action_mut(m, "publish", "smoke").validations[0].kind =
                ValidationKind::SetClosure as i32;
        }),
        ("bound", |m| {
            publish_mut(m, "publish", "publish").attempts = 5;
        }),
        ("channel trust", |m| {
            federated_mut(m).audience = "other.example.com".to_owned();
        }),
        ("channel monotonic", |m| m.channels[0].monotonic = true),
        ("stage environment", |m| {
            stage_mut(m, "build").environment = "other".to_owned();
        }),
    ];
    let base = machine_hash(&good());
    let mut seen = vec![base.to_string()];
    for (what, edit) in &edits {
        let mut m = good();
        edit(&mut m);
        let hash = machine_hash(&m);
        assert_ne!(hash, base, "{what}");
        seen.push(hash.to_string());
    }
    let count = seen.len();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), count, "two edits shared a hash");
}
