// SPDX-License-Identifier: Apache-2.0
//! The wire goldens (`sdk/wire-golden`, written by `ucdk-api/tests/sdk_golden.rs`) are only a
//! contract if they write every field of every message: a field no golden has set could be
//! encoded wrongly by a generated encoder and nobody would see. This reads the goldens through
//! the descriptors, and fails naming each message field that no golden sets.

use std::collections::BTreeSet;
use std::path::PathBuf;

use prost_reflect::{DynamicMessage, MessageDescriptor, ReflectMessage as _, Value};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Records `(message, field)` for every field set in `message`, and in the messages inside it.
fn record(message: &DynamicMessage, seen: &mut BTreeSet<String>) {
    for field in message.descriptor().fields() {
        if !message.has_field(&field) {
            continue;
        }
        seen.insert(field.full_name().to_owned());
        let value = message.get_field(&field);
        let values: Vec<&Value> = match &*value {
            Value::List(items) => items.iter().collect(),
            Value::Map(entries) => entries.values().collect(),
            single => vec![single],
        };
        for v in values {
            if let Value::Message(inner) = v {
                record(inner, seen);
            }
        }
    }
}

/// Every field of `message` and of the messages declared inside it.
fn all_fields(message: &MessageDescriptor, out: &mut BTreeSet<String>) {
    if message.is_map_entry() {
        return;
    }
    out.extend(message.fields().map(|f| f.full_name().to_owned()));
    for child in message.child_messages() {
        all_fields(&child, out);
    }
}

#[test]
fn the_goldens_set_every_field_of_every_message() {
    let pool = ucdk_codegen::compile(&root()).expect("the protos compile");
    let dir = root().join("sdk/wire-golden");
    let manifest = std::fs::read_to_string(dir.join("manifest.txt")).expect("manifest.txt");
    let mut seen = BTreeSet::new();
    for line in manifest.lines() {
        let (file, name) = line.split_once(' ').expect("`<file> <message>`");
        let descriptor = pool
            .get_message_by_name(name)
            .expect("a message of the pool");
        let bytes = std::fs::read(dir.join(file)).expect("a golden");
        let message = DynamicMessage::decode(descriptor, bytes.as_slice()).expect("it decodes");
        record(&message, &mut seen);
    }

    let mut wanted = BTreeSet::new();
    for file in ucdk_codegen::PROTO_FILES {
        for message in pool.get_file_by_name(file).expect("a file").messages() {
            all_fields(&message, &mut wanted);
        }
    }
    // The Duration the resource messages hold is a message of the SDK too.
    all_fields(
        &pool
            .get_message_by_name("google.protobuf.Duration")
            .expect("Duration"),
        &mut wanted,
    );
    let missing: Vec<&String> = wanted.difference(&seen).collect();
    assert!(missing.is_empty(), "no golden sets: {missing:#?}");
    assert!(wanted.len() > 100, "{} fields read", wanted.len());
}
