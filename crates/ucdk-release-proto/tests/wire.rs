// SPDX-License-Identifier: Apache-2.0
//! The messages of `ucdk.release.v1` as wire bytes.
//!
//! `every_field_round_trips` fills every field of every message, in each arm of each oneof,
//! with the descriptor and encodes it; the generated type must read the bytes and write them
//! back to a message equal to the one the descriptor made, so no field is dropped and the
//! generated code and the descriptor agree on every number and type. The streams in the second
//! test are written by hand, byte by byte, with the number the proto declares, because the
//! generated code agrees with itself by construction.

use std::collections::BTreeSet;

use prost::Message;
use protox::Compiler;
use protox::prost_reflect::{
    DescriptorPool, DynamicMessage, Kind, MapKey, MessageDescriptor, Value,
};
use ucdk_release_proto::v1::action::Kind as ActionKind;
use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::*;

const PROTO_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/proto");
const PACKAGE: &str = "ucdk.release.v1.";

fn pool() -> DescriptorPool {
    let mut compiler = Compiler::new([PROTO_ROOT]).expect("compiler");
    compiler
        .open_file("ucdk/release/v1/release.proto")
        .expect("release.proto compiles");
    compiler.descriptor_pool()
}

/// A non-default value of the kind of `field`, distinct per field number.
fn scalar(kind: &Kind, number: u32, arm: usize) -> Value {
    match kind {
        Kind::Bool => Value::Bool(true),
        Kind::Int32 | Kind::Sint32 | Kind::Sfixed32 => Value::I32(number as i32 + 1),
        Kind::Uint32 | Kind::Fixed32 => Value::U32(number + 1),
        Kind::Int64 | Kind::Sint64 | Kind::Sfixed64 => Value::I64(i64::from(number) + 1),
        Kind::Uint64 | Kind::Fixed64 => Value::U64(u64::from(number) + 1),
        Kind::String => Value::String(format!("v{number}")),
        Kind::Enum(e) => Value::EnumNumber(
            e.values()
                .map(|v| v.number())
                .find(|n| *n != 0)
                .expect("a non-zero value"),
        ),
        Kind::Message(m) => Value::Message(fill(m, arm)),
        other => panic!("a kind the proto does not use: {other:?}"),
    }
}

/// `desc` with every field set; of a oneof, the arm `arm` modulo its arms.
fn fill(desc: &MessageDescriptor, arm: usize) -> DynamicMessage {
    let mut msg = DynamicMessage::new(desc.clone());
    for field in desc.fields() {
        if let Some(oneof) = field.containing_oneof() {
            let arms: Vec<_> = oneof.fields().collect();
            if arms[arm % arms.len()].number() != field.number() {
                continue;
            }
        }
        let n = field.number();
        let value = if field.is_map() {
            let Kind::Message(entry) = field.kind() else {
                panic!("a map is a message");
            };
            let value_kind = entry.map_entry_value_field().kind();
            Value::Map(
                [("k1", 1), ("k2", 2)]
                    .into_iter()
                    .map(|(key, at)| {
                        (
                            MapKey::String(key.to_owned()),
                            scalar(&value_kind, n + at, arm),
                        )
                    })
                    .collect(),
            )
        } else if field.is_list() {
            Value::List(vec![
                scalar(&field.kind(), n, arm),
                scalar(&field.kind(), n + 1, arm),
            ])
        } else {
            scalar(&field.kind(), n, arm)
        };
        msg.set_field(&field, value);
    }
    msg
}

/// Decodes `bytes` as `T` and writes it back.
fn reencode<T: Message + Default>(bytes: &[u8]) -> Vec<u8> {
    T::decode(bytes)
        .expect("the generated type reads the bytes")
        .encode_to_vec()
}

macro_rules! through_generated {
    ($name:expr, $bytes:expr; $($t:ident),* $(,)?) => {
        match $name {
            $(concat!("ucdk.release.v1.", stringify!($t)) => reencode::<$t>($bytes),)*
            other => panic!("{other} has no generated type listed in this test"),
        }
    };
}

fn generated(name: &str, bytes: &[u8]) -> Vec<u8> {
    through_generated!(name, bytes;
        Machine, BuildSystem, Target, Stage, Action, Build, Test, Publish, Smoke, Validation,
        ArtifactDecl, Channel, Repository, PushIdentity, Anonymous, Federated, Manifest, Stamped,
        FileMeta, OciMeta, Member, ReleaseSet, Producer, ProducedBy, BuildStamp, Coordinate,
        Held, RunIdentity, SourceFacts, Row, ValidationReport, ActionReport, Effect,
    )
}

#[test]
fn every_field_round_trips() {
    let pool = pool();
    let messages: Vec<MessageDescriptor> = pool
        .all_messages()
        .filter(|m| m.full_name().starts_with(PACKAGE) && !m.is_map_entry())
        .collect();
    assert_eq!(
        messages.len(),
        33,
        "a message was added or removed; list it here and in `generated`"
    );
    for desc in messages {
        // Two arms cover both sides of every oneof of the package.
        for arm in 0..2 {
            let filled = fill(&desc, arm);
            for field in desc.fields() {
                if field.containing_oneof().is_none() {
                    assert!(
                        filled.has_field(&field),
                        "{}.{}",
                        desc.full_name(),
                        field.name()
                    );
                }
            }
            let bytes = filled.encode_to_vec();
            let back = generated(desc.full_name(), &bytes);
            let read = DynamicMessage::decode(desc.clone(), back.as_slice())
                .expect("the descriptor reads them back");
            assert_eq!(read, filled, "{} (arm {arm})", desc.full_name());
        }
    }
}

#[test]
fn every_message_of_the_package_has_a_generated_type() {
    // The macro above lists the generated types; the pool says which messages exist.
    let pool = pool();
    let names: BTreeSet<String> = pool
        .all_messages()
        .filter(|m| m.full_name().starts_with(PACKAGE) && !m.is_map_entry())
        .map(|m| m.full_name().to_owned())
        .collect();
    for name in names {
        // Empty bytes are the empty message of any type, and the call fails on a missing name.
        assert!(generated(&name, &[]).is_empty(), "{name}");
    }
}

#[test]
fn held_decodes_from_literal_bytes() {
    let bytes: Vec<u8> = [
        // 1 digest "d"
        &[0x0A, 0x01, b'd'][..],
        // 2 stamp { 1 build_number 7, 2 source_commit "c" }
        &[0x12, 0x05, 0x08, 0x07, 0x12, 0x01, b'c'],
        // 3 exceptional true
        &[0x18, 0x01],
    ]
    .concat();
    let held = Held::decode(bytes.as_slice()).expect("decodes");
    assert_eq!(
        held,
        Held {
            digest: "d".to_owned(),
            stamp: Some(Stamped {
                build_number: 7,
                source_commit: "c".to_owned()
            }),
            exceptional: true
        }
    );
    assert_eq!(held.encode_to_vec(), bytes);
}

#[test]
fn a_publish_action_decodes_from_literal_bytes() {
    let bytes: Vec<u8> = [
        // 1 name "s"
        &[0x0A, 0x01, b's'][..],
        // 4 publish { 1 platform "p", 2 artifacts "a", 3 channel "c", 4 attempts 5, 5 polls 6 }
        &[
            0x22, 0x0D, 0x0A, 0x01, b'p', 0x12, 0x01, b'a', 0x1A, 0x01, b'c', 0x20, 0x05, 0x28,
            0x06,
        ],
        // 6 validations { 1 name "v", 5 wait_polls 4, 7 kind 2 (CHANNEL_READBACK) }
        &[0x32, 0x07, 0x0A, 0x01, b'v', 0x28, 0x04, 0x38, 0x02],
    ]
    .concat();
    let action = Action::decode(bytes.as_slice()).expect("decodes");
    assert_eq!(action.name, "s");
    assert_eq!(
        action.kind,
        Some(ActionKind::Publish(Publish {
            platform: "p".to_owned(),
            artifacts: vec!["a".to_owned()],
            channel: "c".to_owned(),
            attempts: 5,
            polls: 6
        }))
    );
    assert_eq!(action.validations.len(), 1);
    assert_eq!(
        action.validations[0].kind,
        ValidationKind::ChannelReadback as i32
    );
    assert_eq!(action.validations[0].wait_polls, 4);
    assert_eq!(action.encode_to_vec(), bytes);
}

#[test]
fn a_test_action_decodes_from_literal_bytes() {
    let bytes: Vec<u8> = [
        // 1 name "t"
        &[0x0A, 0x01, b't'][..],
        // 3 test { 1 targets { 1 label "//a:b", 2 flags "-c", 3 env { "K": "V" } } }
        &[
            0x1A, 0x15, 0x0A, 0x13, 0x0A, 0x05, b'/', b'/', b'a', b':', b'b', 0x12, 0x02, b'-',
            b'c', 0x1A, 0x06, 0x0A, 0x01, b'K', 0x12, 0x01, b'V',
        ],
    ]
    .concat();
    let action = Action::decode(bytes.as_slice()).expect("decodes");
    let Some(ActionKind::Test(test)) = &action.kind else {
        panic!("a test action: {action:?}");
    };
    assert_eq!(test.targets.len(), 1);
    assert_eq!(test.targets[0].label, "//a:b");
    assert_eq!(test.targets[0].flags, ["-c"]);
    assert_eq!(test.targets[0].env.get("K").map(String::as_str), Some("V"));
    assert_eq!(action.encode_to_vec(), bytes);
}

#[test]
fn a_federated_identity_decodes_from_literal_bytes() {
    let bytes: Vec<u8> = [
        // 1 subject "s"
        &[0x0A, 0x01, b's'][..],
        // 6 pinned_claims { "k": "v" }
        &[0x32, 0x06, 0x0A, 0x01, b'k', 0x12, 0x01, b'v'],
        // 7 stage_scope "a", 8 commit_scope "b", 9 read_scope "c"
        &[0x3A, 0x01, b'a', 0x42, 0x01, b'b', 0x4A, 0x01, b'c'],
    ]
    .concat();
    let fed = Federated::decode(bytes.as_slice()).expect("decodes");
    assert_eq!(fed.subject, "s");
    assert_eq!(fed.pinned_claims.get("k").map(String::as_str), Some("v"));
    assert_eq!(
        (&*fed.stage_scope, &*fed.commit_scope, &*fed.read_scope),
        ("a", "b", "c")
    );
    assert_eq!(fed.encode_to_vec(), bytes);
}

#[test]
fn a_manifest_with_an_image_arm_decodes_from_literal_bytes() {
    let bytes: Vec<u8> = [
        // 2 type 2 (OCI), 3 name "n", 8 size 300
        &[0x10, 0x02, 0x1A, 0x01, b'n', 0x40, 0xAC, 0x02][..],
        // 11 oci { 1 os "o", 2 cpu "c" }
        &[0x5A, 0x06, 0x0A, 0x01, b'o', 0x12, 0x01, b'c'],
    ]
    .concat();
    let manifest = Manifest::decode(bytes.as_slice()).expect("decodes");
    assert_eq!(manifest.r#type, ArtifactType::Oci as i32);
    assert_eq!(manifest.size, 300);
    assert_eq!(
        manifest.metadata,
        Some(Metadata::Oci(OciMeta {
            os: "o".to_owned(),
            cpu: "c".to_owned()
        }))
    );
    assert_eq!(manifest.encode_to_vec(), bytes);
}

#[test]
fn a_retired_number_is_skipped_not_read_as_another_field() {
    // 5 and 6 (retired in ArtifactDecl) carry varints, 3 (the retired builder) a message; 1
    // name "n" follows. The retired fields are unknown to the type and are dropped, and nothing
    // else moves.
    let bytes = [
        0x28, 0x09, 0x30, 0x01, 0x1A, 0x02, 0x0A, 0x00, 0x0A, 0x01, b'n',
    ];
    let decl = ArtifactDecl::decode(bytes.as_slice()).expect("decodes");
    assert_eq!(
        decl,
        ArtifactDecl {
            name: "n".to_owned(),
            ..ArtifactDecl::default()
        }
    );
    // 2 (the retired validator, a string) and 3 (the retired subject) of a validation.
    let bytes = [0x12, 0x01, b'x', 0x18, 0x02, 0x0A, 0x01, b'v'];
    let validation = Validation::decode(bytes.as_slice()).expect("decodes");
    assert_eq!(
        validation,
        Validation {
            name: "v".to_owned(),
            ..Validation::default()
        }
    );
}
