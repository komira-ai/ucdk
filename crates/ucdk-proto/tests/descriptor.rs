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
    assert!(
        pool.get_message_by_name("kci.resource.v1.Resource")
            .is_none()
    );
}
