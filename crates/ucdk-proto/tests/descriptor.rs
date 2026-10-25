// SPDX-License-Identifier: Apache-2.0
//! The custom options of the catalog, read from the descriptor `protox` builds from the proto.
//! Generated code does not carry extensions, so this is the only place they can be checked.

use protox::Compiler;

const PROTO_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/proto");
const MESSAGE_OPTIONS: &str = "google.protobuf.MessageOptions";

fn pool() -> protox::prost_reflect::DescriptorPool {
    let mut compiler = Compiler::new([PROTO_ROOT]).expect("compiler");
    compiler
        .open_file("ucdk/resource/v1/resource.proto")
        .expect("resource.proto compiles");
    compiler.descriptor_pool()
}

#[test]
fn custom_options_50001_50002_present_in_descriptor() {
    let pool = pool();
    let portability = pool
        .get_extension_by_name("ucdk.resource.v1.portability")
        .expect("portability extension");
    let exposes = pool
        .get_extension_by_name("ucdk.resource.v1.exposes")
        .expect("exposes extension");
    assert_eq!(portability.number(), 50001);
    assert_eq!(exposes.number(), 50002);
    assert_eq!(
        portability.containing_message().full_name(),
        MESSAGE_OPTIONS
    );
    assert_eq!(exposes.containing_message().full_name(), MESSAGE_OPTIONS);
    assert!(exposes.is_list());

    let service = pool
        .get_message_by_name("ucdk.resource.v1.Service")
        .expect("Service")
        .options();
    // PORTABLE is 1; URL is 1 and HOST is 2.
    assert_eq!(
        service.get_extension(&portability).as_enum_number(),
        Some(1)
    );
    let exposed: Vec<Option<i32>> = service
        .get_extension(&exposes)
        .as_list()
        .expect("a list")
        .iter()
        .map(protox::prost_reflect::Value::as_enum_number)
        .collect();
    assert_eq!(exposed, [Some(1), Some(2)]);

    let job = pool
        .get_message_by_name("ucdk.resource.v1.Job")
        .expect("Job")
        .options();
    assert_eq!(job.get_extension(&portability).as_enum_number(), Some(1));
    assert!(!job.has_extension(&exposes));

    // The options mark primitives only.
    let resource = pool
        .get_message_by_name("ucdk.resource.v1.Resource")
        .expect("Resource")
        .options();
    assert!(!resource.has_extension(&portability));
    assert!(!resource.has_extension(&exposes));
}

#[test]
fn package_and_reserved_names_are_as_declared() {
    let pool = pool();
    let resource = pool
        .get_message_by_name("ucdk.resource.v1.Resource")
        .expect("Resource");
    let reserved: Vec<&str> = resource.reserved_names().collect();
    assert_eq!(reserved, ["only_in"]);
    let ranges: Vec<_> = resource
        .reserved_ranges()
        .map(|r| (r.start, r.end))
        .collect();
    assert_eq!(ranges, [(4, 5)]);
    let outside: Vec<String> = pool
        .all_messages()
        .filter(|m| m.parent_file().name().starts_with("ucdk/"))
        .map(|m| m.full_name().to_owned())
        .filter(|name| !name.starts_with("ucdk.resource.v1."))
        .collect();
    assert!(
        outside.is_empty(),
        "messages outside the package: {outside:?}"
    );
}

/// The text of one string option on a field.
fn text_option(
    pool: &protox::prost_reflect::DescriptorPool,
    message: &str,
    field: &str,
    option: &str,
) -> Option<String> {
    let ext = pool
        .get_extension_by_name(&format!("ucdk.resource.v1.{option}"))
        .expect("the option is defined");
    let options = pool
        .get_message_by_name(&format!("ucdk.resource.v1.{message}"))
        .expect("message")
        .get_field_by_name(field)
        .expect("field")
        .options();
    options
        .has_extension(&ext)
        .then(|| options.get_extension(&ext).as_str().map(str::to_owned))
        .flatten()
}

/// Proves: the lowering options sit on the fields they were designed for, with the numbers 50011
/// to 50015 on `FieldOptions`, and the versioned defaults of the two v1 types are the ones the
/// plan hashes were pinned with.
#[test]
fn lowering_options_are_declared_on_the_v1_types() {
    let pool = pool();
    for (name, number) in [
        ("node_name", 50011),
        ("render", 50012),
        ("fallback", 50013),
        ("role", 50014),
        ("unlowered", 50015),
    ] {
        let ext = pool
            .get_extension_by_name(&format!("ucdk.resource.v1.{name}"))
            .expect("option");
        assert_eq!(ext.number(), number, "{name}");
        assert_eq!(
            ext.containing_message().full_name(),
            "google.protobuf.FieldOptions"
        );
    }
    let fallback = |m, f| text_option(&pool, m, f, "fallback");
    assert_eq!(fallback("Service", "port").as_deref(), Some("8080"));
    assert_eq!(fallback("Service", "size").as_deref(), Some("1000m/512MB"));
    assert_eq!(fallback("Service", "scale").as_deref(), Some("0..10"));
    assert_eq!(
        fallback("Service", "request_timeout").as_deref(),
        Some("60s0n")
    );
    assert_eq!(fallback("Job", "size").as_deref(), Some("1000m/512MB"));
    assert_eq!(fallback("Job", "timeout").as_deref(), Some("600s0n"));
    assert_eq!(fallback("Job", "max_retries"), None);
    let role = |m, f| text_option(&pool, m, f, "role");
    assert_eq!(role("Service", "public").as_deref(), Some("public"));
    assert_eq!(role("Job", "schedule").as_deref(), Some("schedule"));
    assert_eq!(role("Service", "internal"), None);
    assert_eq!(
        text_option(&pool, "Service", "args", "node_name").as_deref(),
        Some("arg")
    );
    assert_eq!(
        text_option(&pool, "Job", "args", "node_name").as_deref(),
        Some("arg")
    );
}

/// Proves: the descriptor set the planner reads (built with the crate) defines the same types
/// and keeps the custom options: a pool decoded from it marks `Service` `PORTABLE`.
#[test]
fn the_embedded_descriptor_set_keeps_the_custom_options() {
    let pool = protox::prost_reflect::DescriptorPool::decode(ucdk_proto::DESCRIPTOR_SET)
        .expect("a descriptor set");
    let portability = pool
        .get_extension_by_name("ucdk.resource.v1.portability")
        .expect("portability");
    let service = pool
        .get_message_by_name("ucdk.resource.v1.Service")
        .expect("Service")
        .options();
    assert_eq!(
        service.get_extension(&portability).as_enum_number(),
        Some(1)
    );
}
