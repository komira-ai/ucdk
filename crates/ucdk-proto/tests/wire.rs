// SPDX-License-Identifier: Apache-2.0
//! The field numbers of `ucdk.resource.v1` as wire bytes.
//!
//! A field number is what is stored: renumbering a field is legal to the compiler and decodes
//! stored data as the wrong field. Every stream below is written by hand, byte by byte, with the
//! number the proto declares; none is produced by the generated code, which would agree with
//! itself by construction. A decoded message is read back field by field (this catches two fields
//! of one wire type trading numbers, which a round trip does not), and a stream that is fully
//! understood re-encodes to the same number of bytes (this catches a field that moved to a number
//! nothing reads).

use prost::Message;
use ucdk_proto::v1::{
    Access, Image, Job, Output, Portability, Ref, Resource, ResourceList, Service, Value, image,
    job, r#ref, resource, service, value,
};

fn decode<M: Message + Default>(bytes: &[u8]) -> M {
    M::decode(bytes).expect("the stream decodes")
}

/// Decodes `bytes`, and checks that nothing in it was dropped: re-encoding gives as many bytes.
fn decode_whole<M: Message + Default + PartialEq + std::fmt::Debug>(bytes: &[u8]) -> M {
    let message: M = decode(bytes);
    assert_eq!(message.encoded_len(), bytes.len(), "a field was dropped");
    assert_eq!(decode::<M>(&message.encode_to_vec()), message);
    message
}

// ---- Messages: every declared field, by number -----------------------------------------------

#[test]
fn service_decodes_from_literal_bytes() {
    let bytes: Vec<u8> = [
        // 1 image { 1 output { 1 step "build", 2 name "img" }, 3 platform "linux/amd64" }
        &[0x0A, 0x1B][..],
        &[
            0x0A, 0x0C, 0x0A, 0x05, b'b', b'u', b'i', b'l', b'd', 0x12, 0x03, b'i', b'm', b'g',
        ],
        &[
            0x1A, 0x0B, b'l', b'i', b'n', b'u', b'x', b'/', b'a', b'm', b'd', b'6', b'4',
        ],
        // 2 port 8080
        &[0x10, 0x90, 0x3F],
        // 3 args "a1", "a2"
        &[0x1A, 0x02, b'a', b'1', 0x1A, 0x02, b'a', b'2'],
        // 4 env: "K" literal "v", "P" param "p", "R" ref { resource "db", standard 1 }
        &[0x22, 0x08, 0x0A, 0x01, b'K', 0x12, 0x03, 0x0A, 0x01, b'v'],
        &[0x22, 0x08, 0x0A, 0x01, b'P', 0x12, 0x03, 0x12, 0x01, b'p'],
        &[
            0x22, 0x0D, 0x0A, 0x01, b'R', 0x12, 0x08, 0x1A, 0x06, 0x0A, 0x02, b'd', b'b', 0x10,
            0x01,
        ],
        // 5 size { 1 cpu_millis 500, 2 memory_mb 256 }
        &[0x2A, 0x06, 0x08, 0xF4, 0x03, 0x10, 0x80, 0x02],
        // 6 scale { 1 min 0 (written), 2 max 10 }
        &[0x32, 0x04, 0x08, 0x00, 0x10, 0x0A],
        // 7 health_path "/hlz"
        &[0x3A, 0x04, b'/', b'h', b'l', b'z'],
        // 8 request_timeout { 1 seconds 30, 2 nanos 5 }
        &[0x42, 0x04, 0x08, 0x1E, 0x10, 0x05],
        // 9 max_concurrency 80
        &[0x48, 0x50],
        // 10 public {}
        &[0x52, 0x00],
        // 12 secret_env: "S" { 1 name "sec", 2 store "st", 3 version "7" }
        &[0x62, 0x11, 0x0A, 0x01, b'S', 0x12, 0x0C],
        &[
            0x0A, 0x03, b's', b'e', b'c', 0x12, 0x02, b's', b't', 0x1A, 0x01, b'7',
        ],
    ]
    .concat();

    let svc: Service = decode_whole(&bytes);

    let img = svc.image.as_ref().expect("image");
    assert_eq!(img.platform, "linux/amd64");
    let Some(image::Source::Output(step)) = &img.source else {
        panic!("image source is not a step output: {:?}", img.source);
    };
    assert_eq!((step.step.as_str(), step.name.as_str()), ("build", "img"));
    assert_eq!(svc.port, 8080);
    assert_eq!(svc.args, ["a1", "a2"]);
    assert_eq!(svc.env.len(), 3);
    assert_eq!(svc.env["K"].v, Some(value::V::Literal("v".to_owned())));
    assert_eq!(svc.env["P"].v, Some(value::V::Param("p".to_owned())));
    let Some(value::V::Ref(db)) = &svc.env["R"].v else {
        panic!("env R is not a ref");
    };
    assert_eq!(db.resource, "db");
    assert_eq!(db.output, Some(r#ref::Output::Standard(Output::Url as i32)));
    let size = svc.size.expect("size");
    assert_eq!((size.cpu_millis, size.memory_mb), (500, 256));
    let scale = svc.scale.expect("scale");
    assert_eq!((scale.min, scale.max), (Some(0), 10));
    assert_eq!(svc.health_path, "/hlz");
    let timeout = svc.request_timeout.expect("request_timeout");
    assert_eq!((timeout.seconds, timeout.nanos), (30, 5));
    assert_eq!(svc.max_concurrency, 80);
    assert_eq!(
        svc.exposure,
        Some(service::Exposure::Public(service::Public {}))
    );
    let secret = &svc.secret_env["S"];
    assert_eq!(secret.name, "sec");
    assert_eq!(secret.store.as_deref(), Some("st"));
    assert_eq!(secret.version.as_deref(), Some("7"));
}

#[test]
fn service_exposure_internal_decodes_from_literal_bytes() {
    let svc: Service = decode_whole(&[0x5A, 0x00]);
    assert_eq!(
        svc.exposure,
        Some(service::Exposure::Internal(service::Internal {}))
    );
}

#[test]
fn job_decodes_from_literal_bytes() {
    let bytes: Vec<u8> = [
        // 1 image { 2 digest "sha256:ab" }
        &[
            0x0A, 0x0B, 0x12, 0x09, b's', b'h', b'a', b'2', b'5', b'6', b':', b'a', b'b',
        ][..],
        // 2 args "go"
        &[0x12, 0x02, b'g', b'o'],
        // 3 size { 1 cpu_millis 1000, 2 memory_mb 512 }
        &[0x1A, 0x06, 0x08, 0xE8, 0x07, 0x10, 0x80, 0x04],
        // 4 max_retries 0 (written)
        &[0x20, 0x00],
        // 5 timeout { 1 seconds 600 }
        &[0x2A, 0x03, 0x08, 0xD8, 0x04],
        // 6 env: "E" literal "e"
        &[0x32, 0x08, 0x0A, 0x01, b'E', 0x12, 0x03, 0x0A, 0x01, b'e'],
        // 7 secret_env: "T" { 1 name "tok" }
        &[
            0x3A, 0x0A, 0x0A, 0x01, b'T', 0x12, 0x05, 0x0A, 0x03, b't', b'o', b'k',
        ],
        // 11 schedule { 1 cron "0 * * * *", 2 timezone "UTC" }
        &[
            0x5A, 0x10, 0x0A, 0x09, b'0', b' ', b'*', b' ', b'*', b' ', b'*', b' ', b'*',
        ],
        &[0x12, 0x03, b'U', b'T', b'C'],
    ]
    .concat();

    let job: Job = decode_whole(&bytes);

    assert_eq!(
        job.image,
        Some(Image {
            source: Some(image::Source::Digest("sha256:ab".to_owned())),
            platform: String::new(),
        })
    );
    assert_eq!(job.args, ["go"]);
    let size = job.size.expect("size");
    assert_eq!((size.cpu_millis, size.memory_mb), (1000, 512));
    assert_eq!(job.max_retries, Some(0));
    assert_eq!(job.timeout.expect("timeout").seconds, 600);
    assert_eq!(job.env["E"].v, Some(value::V::Literal("e".to_owned())));
    assert_eq!(job.secret_env["T"].name, "tok");
    let Some(job::Trigger::Schedule(schedule)) = &job.trigger else {
        panic!("trigger is not a schedule: {:?}", job.trigger);
    };
    assert_eq!(
        (schedule.cron.as_str(), schedule.timezone.as_str()),
        ("0 * * * *", "UTC")
    );
}

#[test]
fn job_trigger_on_demand_decodes_from_literal_bytes() {
    let job: Job = decode_whole(&[0x52, 0x00]);
    assert_eq!(job.trigger, Some(job::Trigger::OnDemand(job::OnDemand {})));
}

#[test]
fn resource_list_round_trip() {
    // list { 1 resource { 1 id "api", 2 uses { 1 target { 1 resource "db" }, 2 access 1 },
    //                     10 service { 2 port 8080 } },
    //        1 resource { 1 id "nightly", 11 job { 4 max_retries 3 } } }
    let bytes: &[u8] = &[
        0x0A, 0x14, 0x0A, 0x03, b'a', b'p', b'i', 0x12, 0x08, 0x0A, 0x04, 0x0A, 0x02, b'd', b'b',
        0x10, 0x01, 0x52, 0x03, 0x10, 0x90, 0x3F, //
        0x0A, 0x0D, 0x0A, 0x07, b'n', b'i', b'g', b'h', b't', b'l', b'y', 0x5A, 0x02, 0x20, 0x03,
    ];

    let list: ResourceList = decode_whole(bytes);

    assert_eq!(list.resource.len(), 2);
    let api = &list.resource[0];
    assert_eq!(api.id, "api");
    assert_eq!(api.uses.len(), 1);
    assert_eq!(api.uses[0].target.as_ref().expect("target").resource, "db");
    assert_eq!(api.uses[0].access, Access::Call as i32);
    let Some(resource::Body::Service(service)) = &api.body else {
        panic!("api is not a service");
    };
    assert_eq!(service.port, 8080);
    let nightly = &list.resource[1];
    assert_eq!(nightly.id, "nightly");
    let Some(resource::Body::Job(job)) = &nightly.body else {
        panic!("nightly is not a job");
    };
    assert_eq!(job.max_retries, Some(3));
    // Fields written in ascending order encode back to the very same bytes.
    assert_eq!(list.encode_to_vec(), bytes);
}

#[test]
fn enum_ordinals_match_both_ways() {
    let outputs = [(0, Output::Unset), (1, Output::Url), (2, Output::Host)];
    for (number, variant) in outputs {
        assert_eq!(variant as i32, number);
        assert_eq!(Output::try_from(number), Ok(variant));
    }
    let accesses = [(0, Access::Unset), (1, Access::Call)];
    for (number, variant) in accesses {
        assert_eq!(variant as i32, number);
        assert_eq!(Access::try_from(number), Ok(variant));
    }
    let portabilities = [
        (0, Portability::Unset),
        (1, Portability::Portable),
        (2, Portability::CloudBound),
    ];
    for (number, variant) in portabilities {
        assert_eq!(variant as i32, number);
        assert_eq!(Portability::try_from(number), Ok(variant));
    }
    // The numbers held for later values are not declared.
    assert!((3..=5).all(|n| Output::try_from(n).is_err()));
    assert!((2..=6).all(|n| Access::try_from(n).is_err()));
    assert!(Portability::try_from(3).is_err());
    // A name maps to its number and back.
    assert_eq!(Output::from_str_name("HOST"), Some(Output::Host));
    assert_eq!(Access::Call.as_str_name(), "CALL");
}

#[test]
fn ref_oneof_standard_and_named() {
    // 1 resource "db", 2 standard HOST
    let standard: Ref = decode_whole(&[0x0A, 0x02, b'd', b'b', 0x10, 0x02]);
    assert_eq!(standard.resource, "db");
    assert_eq!(
        standard.output,
        Some(r#ref::Output::Standard(Output::Host as i32))
    );

    // 1 resource "db", 3 named "cert"
    let named: Ref = decode_whole(&[0x0A, 0x02, b'd', b'b', 0x1A, 0x04, b'c', b'e', b'r', b't']);
    assert_eq!(named.output, Some(r#ref::Output::Named("cert".to_owned())));

    // Neither arm: the reference names the resource itself.
    let itself: Ref = decode_whole(&[0x0A, 0x02, b'd', b'b']);
    assert_eq!(itself.output, None);

    // Both arms on the wire: the arm written last wins, as for every oneof.
    let named_last: Ref = decode(&[0x10, 0x01, 0x1A, 0x01, b'x']);
    assert_eq!(
        named_last.output,
        Some(r#ref::Output::Named("x".to_owned()))
    );
    let standard_last: Ref = decode(&[0x1A, 0x01, b'x', 0x10, 0x01]);
    assert_eq!(
        standard_last.output,
        Some(r#ref::Output::Standard(Output::Url as i32))
    );
}

// ---- Held and unknown numbers ----------------------------------------------------------------

/// One length-delimited field `number` holding the single byte `x`.
fn held_field(number: u32) -> Vec<u8> {
    // The tag is (number << 3) | 2, as a varint: written out per number, not computed.
    let tag: &[u8] = match number {
        3 => &[0x1A],
        5 => &[0x2A],
        6 => &[0x32],
        12 => &[0x62],
        13 => &[0x6A],
        14 => &[0x72],
        15 => &[0x7A],
        16 => &[0x82, 0x01],
        17 => &[0x8A, 0x01],
        18 => &[0x92, 0x01],
        90 => &[0xD2, 0x05],
        other => panic!("no tag written for {other}"),
    };
    [tag, &[0x01, b'x']].concat()
}

#[test]
fn held_field_numbers_3_5_6_12_to_18_90_are_ignored_on_decode() {
    for number in [3, 5, 6, 12, 13, 14, 15, 16, 17, 18, 90] {
        // 1 id "a", then the held field.
        let bytes = [&[0x0A, 0x01, b'a'][..], &held_field(number)].concat();
        let resource: Resource = decode(&bytes);
        assert_eq!(resource.id, "a", "held {number}");
        assert!(resource.uses.is_empty(), "held {number}");
        assert_eq!(resource.body, None, "held {number}");
        assert_eq!(
            resource.encode_to_vec(),
            [0x0A, 0x01, b'a'],
            "held {number}"
        );
    }
}

#[test]
fn retired_resource_field_4_is_ignored_on_decode() {
    // 1 id "a", 4 (retired) holding the byte `x`: it fills no field.
    let resource: Resource = decode(&[0x0A, 0x01, b'a', 0x22, 0x01, b'x']);
    assert_eq!(resource.id, "a");
    assert_eq!(resource.encode_to_vec(), [0x0A, 0x01, b'a']);
}

#[test]
fn held_numbers_of_value_image_service_job_are_ignored_on_decode() {
    // Value 4, Image 4, Service 13 and Job 8 are held; each is a length-delimited byte `x`.
    let value: Value = decode(&[0x22, 0x01, b'x']);
    assert_eq!(value.v, None);
    assert!(value.encode_to_vec().is_empty());

    let image: Image = decode(&[0x22, 0x01, b'x']);
    assert_eq!(image.source, None);
    assert!(image.encode_to_vec().is_empty());

    let service: Service = decode(&[0x6A, 0x01, b'x']);
    assert_eq!(service, Service::default());
    assert!(service.encode_to_vec().is_empty());

    let job: Job = decode(&[0x42, 0x01, b'x']);
    assert_eq!(job, Job::default());
    assert!(job.encode_to_vec().is_empty());
}

#[test]
fn unknown_oneof_arm_is_dropped_not_misread() {
    // Arm 19 is not declared. Its payload is a well-formed service (2 port 8080); read as the
    // service arm it would fill `body`.
    let resource: Resource = decode(&[0x0A, 0x01, b'a', 0x9A, 0x01, 0x03, 0x10, 0x90, 0x3F]);
    assert_eq!(resource.id, "a");
    assert_eq!(resource.body, None);
    assert_eq!(resource.encode_to_vec(), [0x0A, 0x01, b'a']);

    // An unknown arm beside a known one leaves the known one alone, in either order.
    let service_arm = [0x52, 0x03, 0x10, 0x90, 0x3F];
    let unknown_arm = [0x62, 0x01, b'x'];
    for bytes in [
        [&service_arm[..], &unknown_arm].concat(),
        [&unknown_arm[..], &service_arm].concat(),
    ] {
        let resource: Resource = decode(&bytes);
        let Some(resource::Body::Service(service)) = &resource.body else {
            panic!("the service arm was lost");
        };
        assert_eq!(service.port, 8080);
        assert_eq!(resource.encode_to_vec(), service_arm);
    }

    // Value arm 4 is undeclared: the value stays unset instead of becoming a literal.
    let value: Value = decode(&[0x22, 0x01, b'x']);
    assert_eq!(value.v, None);
}
