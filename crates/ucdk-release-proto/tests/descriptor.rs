// SPDX-License-Identifier: Apache-2.0
//! The shape of the `ucdk.release.v1` descriptor: what a renumbered, retyped or dropped field
//! would change, and the numbers that were retired.
//!
//! A field number is what is stored: renumbering a field is legal to the compiler and decodes
//! stored data as the wrong field. `fields.txt` lists every field of the package with its
//! number, kind and cardinality, and every enum value; a change to the proto that is not a
//! change to that file fails here, and a change to that file is a change a reviewer sees.

use protox::Compiler;
use protox::prost_reflect::{DescriptorPool, Kind};

const PROTO_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/proto");
const PACKAGE: &str = "ucdk.release.v1.";

fn pool() -> DescriptorPool {
    let mut compiler = Compiler::new([PROTO_ROOT]).expect("compiler");
    compiler
        .open_file("ucdk/release/v1/release.proto")
        .expect("release.proto compiles");
    compiler.descriptor_pool()
}

fn message(pool: &DescriptorPool, name: &str) -> protox::prost_reflect::MessageDescriptor {
    pool.get_message_by_name(&format!("{PACKAGE}{name}"))
        .expect("the message exists")
}

fn kind_text(kind: &Kind) -> String {
    match kind {
        Kind::Message(m) => m.full_name().to_owned(),
        Kind::Enum(e) => e.full_name().to_owned(),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// Every field and every enum value of the package, one per line, in a stable order.
fn table(pool: &DescriptorPool) -> String {
    let mut lines = Vec::new();
    for m in pool
        .all_messages()
        .filter(|m| m.full_name().starts_with(PACKAGE))
    {
        if m.is_map_entry() {
            continue;
        }
        for f in m.fields() {
            let shape = if f.is_map() {
                "map"
            } else if f.is_list() {
                "repeated"
            } else {
                "single"
            };
            let oneof = f
                .containing_oneof()
                .map_or(String::new(), |o| format!(" oneof {}", o.name()));
            lines.push(format!(
                "{}.{} = {} {} {}{}",
                m.full_name(),
                f.name(),
                f.number(),
                kind_text(&f.kind()),
                shape,
                oneof
            ));
        }
    }
    for e in pool
        .all_enums()
        .filter(|e| e.full_name().starts_with(PACKAGE))
    {
        for v in e.values() {
            lines.push(format!("{} {} = {}", e.full_name(), v.name(), v.number()));
        }
    }
    lines.sort();
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

#[test]
fn every_field_keeps_its_number_name_and_type() {
    assert_eq!(table(&pool()), include_str!("fields.txt"));
}

#[test]
fn retired_numbers_are_reserved() {
    let pool = pool();
    let reserved = |name: &str| -> Vec<(u32, u32)> {
        message(&pool, name)
            .reserved_ranges()
            .map(|r| (r.start, r.end))
            .collect()
    };
    // Each range is half-open: `reserved 5` is the range 5 to 6.
    assert_eq!(reserved("ArtifactDecl"), [(5, 6)]);
    assert_eq!(reserved("OciMeta"), [(3, 4)]);
    let verdict = pool
        .get_enum_by_name(&format!("{PACKAGE}Verdict"))
        .expect("Verdict");
    let ranges: Vec<(i32, i32)> = verdict
        .reserved_ranges()
        .map(|r| (*r.start(), *r.end()))
        .collect();
    assert_eq!(ranges, [(10, 10)]);
    // And nothing declared uses a retired number.
    assert!(message(&pool, "ArtifactDecl").get_field(5).is_none());
    assert!(message(&pool, "OciMeta").get_field(3).is_none());
    assert!(verdict.values().all(|v| v.number() != 10));
    // The two retired fields were not the last of their messages: later numbers stay in use.
    assert!(message(&pool, "ArtifactDecl").get_field(6).is_some());
}

#[test]
fn a_member_carries_no_store_key() {
    let pool = pool();
    let names: Vec<String> = message(&pool, "Member")
        .fields()
        .map(|f| f.name().to_owned())
        .collect();
    assert_eq!(names, ["manifest"]);
    // Nor does any other message of the package carry one: a key is derived on every read.
    for m in pool
        .all_messages()
        .filter(|m| m.full_name().starts_with(PACKAGE) && !m.is_map_entry())
    {
        for f in m.fields() {
            assert!(
                !f.name().contains("key") && !f.name().contains("path"),
                "{}.{} looks like a store key",
                m.full_name(),
                f.name()
            );
        }
    }
}

#[test]
fn every_enum_has_a_zero_value_named_unspecified() {
    let pool = pool();
    let mut seen = 0;
    for e in pool
        .all_enums()
        .filter(|e| e.full_name().starts_with(PACKAGE))
    {
        let zero = e.get_value(0).expect("a zero value");
        assert!(
            zero.name().ends_with("_UNSPECIFIED"),
            "{}: {}",
            e.full_name(),
            zero.name()
        );
        seen += 1;
    }
    assert_eq!(
        seen, 9,
        "an enum was added or removed; this count and fields.txt say so"
    );
}

#[test]
fn every_message_is_in_the_release_package() {
    let pool = pool();
    let outside: Vec<String> = pool
        .all_messages()
        .filter(|m| m.parent_file().name().starts_with("ucdk/"))
        .map(|m| m.full_name().to_owned())
        .filter(|name| !name.starts_with(PACKAGE))
        .collect();
    assert!(
        outside.is_empty(),
        "messages outside the package: {outside:?}"
    );
}

#[test]
fn no_enum_value_is_named_as_a_value_of_another_enum() {
    // Enum values are siblings of their enum in the package, so a name used twice does not
    // compile; this keeps the message of that failure from being the first sign of it.
    let pool = pool();
    let mut names: Vec<String> = pool
        .all_enums()
        .filter(|e| e.full_name().starts_with(PACKAGE))
        .flat_map(|e| e.values().map(|v| v.name().to_owned()).collect::<Vec<_>>())
        .collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), total);
}
