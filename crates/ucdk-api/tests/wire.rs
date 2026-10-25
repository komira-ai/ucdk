// SPDX-License-Identifier: Apache-2.0
//! The messages as bytes: what a host sends and gets back is the proto3 binary format, and the
//! same message always encodes to the same bytes.

mod common;

use common::{World, list_of, lowered, service_of};
use prost::Message as _;
use ucdk_api::v1::{
    ApplyRequest, CredentialSource, ExitClass, Mode, Outcome, PlanRequest, SessionConfig, outcome,
};
use ucdk_engine::Mode as EngineMode;
use ucdk_provider_fake::Op;

#[test]
fn a_session_config_round_trips_and_encodes_the_same_every_time() {
    let config = SessionConfig {
        provider: "fake".to_owned(),
        provider_options: [
            ("region".to_owned(), "r1".to_owned()),
            ("a".to_owned(), "b".to_owned()),
        ]
        .into_iter()
        .collect(),
        ledger_kind: "memory".to_owned(),
        ledger_location: String::new(),
        cell: "cell-one".to_owned(),
        credentials: CredentialSource::Profile as i32,
        profile: "work".to_owned(),
    };
    let bytes = config.encode_to_vec();
    assert_eq!(
        SessionConfig::decode(bytes.as_slice()).expect("decodes"),
        config
    );
    assert_eq!(config.encode_to_vec(), bytes);
}

#[test]
fn a_plan_then_an_apply_through_bytes_ends_in_a_report_with_exit_class_done() {
    let list = list_of(vec![service_of("web", 80), service_of("db", 5432)]);
    let mut world = World::new();

    // The host receives bytes, answers them, and the caller reads the answer from bytes.
    let sent = PlanRequest {
        resources: Some(list.clone()),
        mode: Mode::Apply as i32,
    }
    .encode_to_vec();
    let request = PlanRequest::decode(sent.as_slice())
        .expect("decodes")
        .to_request()
        .expect("a request");
    let answer = Outcome::from(&world.handle(request)).encode_to_vec();
    let planned = Outcome::decode(answer.as_slice()).expect("decodes");
    let Some(outcome::Result::Planned(plan)) = planned.result else {
        panic!("not planned");
    };
    assert_eq!(plan.steps.len(), 2);

    let sent = ApplyRequest {
        resources: Some(list.clone()),
        plan_hash: plan.hash.clone(),
        rollback: false,
    }
    .encode_to_vec();
    let approved = ApplyRequest::decode(sent.as_slice())
        .expect("decodes")
        .approved()
        .expect("approved");
    let desired = lowered(&list);
    let replanned = world.plan(&desired, EngineMode::Apply);
    let answer = Outcome::from(&world.handle(approved.apply(replanned))).encode_to_vec();
    let done = Outcome::decode(answer.as_slice()).expect("decodes");
    assert_eq!(done.exit, ExitClass::Done as i32);
    let Some(outcome::Result::Done(report)) = done.result else {
        panic!("not done");
    };
    assert_eq!(report.created.len(), 2);
}

#[test]
fn an_apply_under_a_stale_hash_is_refused_with_exit_class_refused_and_changes_nothing() {
    let list = list_of(vec![service_of("web", 80)]);
    let mut world = World::new();
    let desired = lowered(&list);
    let plan = world.plan(&desired, EngineMode::Apply);
    let approved = ApplyRequest {
        resources: Some(list),
        plan_hash: "00".repeat(32),
        rollback: false,
    }
    .approved()
    .expect("approved");
    let wire = Outcome::from(&world.handle(approved.apply(plan)));
    assert_eq!(wire.exit, ExitClass::Refused as i32);
    assert!(matches!(wire.result, Some(outcome::Result::Refused(_))));
    assert!(
        world
            .fake
            .calls()
            .iter()
            .all(|c| !matches!(c.op, Op::Create | Op::Update | Op::Delete)),
        "{:?}",
        world.fake.calls()
    );
}
