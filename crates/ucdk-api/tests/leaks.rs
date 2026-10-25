// SPDX-License-Identifier: Apache-2.0
//! The leaks request and response: the request is the engine's read-only sweep, the engine's
//! leaks become a message that keeps every field, and the exit class is the one table's: done
//! when there are none, `leaks` (8) when there are some, and done for an apply that finds some.

mod common;

use common::{World, list_of, lowered, path, service_of};
use prost::Message as _;
use ucdk_api::exit_class;
use ucdk_api::v1::{self, ErrorCode, ExitClass, LeaksRequest, outcome};
use ucdk_engine::{EngineError, Leak, Mode, Outcome, Report, Request};
use ucdk_provider::{Kind, PhysicalId, Provider as _, ProviderError, Stamp, kit};
use ucdk_provider_fake::Op;

fn leak() -> Leak {
    Leak {
        kind: Kind::service(),
        physical: PhysicalId::new("svc-1"),
        name: "ghost-run".to_owned(),
        owner: path("ghost/run").owner(),
        node: path("ghost/run").key(),
    }
}

/// Creates a stamped object nobody records, as a run that lost its ledger would leave.
fn orphan(world: &mut World, id: &str) -> PhysicalId {
    let list = list_of(vec![service_of(id, 80)]);
    let node = lowered(&list).nodes()[0].node.clone();
    let stamp = Stamp::for_node(&kit::cell(), node.path());
    world.fake.create(&node, &stamp).expect("create").physical
}

/// Proves: a leaks request is the engine's sweep and nothing else, whatever bytes it was
/// decoded from. Catches a request that plans, applies or deletes.
#[test]
fn a_leaks_request_becomes_the_engine_sweep() {
    let request =
        LeaksRequest::decode(LeaksRequest {}.encode_to_vec().as_slice()).expect("decodes");
    assert!(matches!(request.to_request(), Request::Leaks));
}

/// Proves: every field of a leak reaches the message as the engine had it. Catches a field
/// dropped or two fields swapped in conversion.
#[test]
fn a_leak_keeps_every_field() {
    let message = v1::Leak::from(&leak());
    assert_eq!(message.kind, "service");
    assert_eq!(message.physical_id, "svc-1");
    assert_eq!(message.name, "ghost-run");
    assert_eq!(message.owner, "ghost");
    assert_eq!(message.node, path("ghost/run").key().to_string());
}

/// Proves: the exit class of a sweep: none found is done, some found is `leaks` with code 8;
/// an apply whose report carries leaks is still done; a sweep that could not read is failed
/// with no effect, whatever the provider's error. Catches leaks mapped to the apply's class, and
/// a read failure that tells the caller to re-plan.
#[test]
fn the_exit_class_of_a_sweep_is_done_leaks_or_failed_with_no_effect() {
    assert_eq!(exit_class(&Outcome::Leaks(Vec::new())), ExitClass::Done);
    assert_eq!(exit_class(&Outcome::Leaks(vec![leak()])), ExitClass::Leaks);
    assert_eq!(ExitClass::Leaks.code(), 8);

    let swept = Report {
        leaks: vec![leak()],
        ..Report::default()
    };
    assert_eq!(exit_class(&Outcome::Done(swept)), ExitClass::Done);

    let failed = Outcome::Failed(ucdk_engine::Failure {
        report: Report::default(),
        at: None,
        error: EngineError::Provider(ProviderError::Unknown("timed out".to_owned())),
    });
    assert_eq!(exit_class(&failed), ExitClass::FailedNoEffect);
}

/// Proves: a sweep through bytes finds the leaked object among recorded ones, answers with the
/// leaks arm and exit class leaks, is stable under encode and decode, and makes no call but the
/// one list. Catches a response that drops the leak, reads the exit from the contents, or a
/// request that mutates.
#[test]
fn a_sweep_through_bytes_answers_the_leaks_with_exit_class_leaks() {
    let mut world = World::new();
    let list = list_of(vec![service_of("web", 80)]);
    let desired = lowered(&list);
    let plan = world.plan(&desired, Mode::Apply);
    let hash = plan.hash().clone();
    world.handle(Request::Apply {
        desired,
        plan,
        plan_hash: hash,
        rollback: false,
    });
    let ghost = orphan(&mut world, "ghost");
    world.fake.clear_calls();

    let answered = world.handle(LeaksRequest {}.to_request());
    let response = v1::Outcome::from(&answered);
    assert_eq!(response.exit, ExitClass::Leaks as i32);
    let Some(outcome::Result::Leaks(found)) = &response.result else {
        panic!("not leaks: {response:?}");
    };
    assert_eq!(found.leaks.len(), 1);
    assert_eq!(found.leaks[0].physical_id, ghost.as_str());
    assert_eq!(found.leaks[0].owner, "ghost");
    let bytes = response.encode_to_vec();
    assert_eq!(
        v1::Outcome::decode(bytes.as_slice()).expect("decodes"),
        response
    );
    let ops: Vec<Op> = world.fake.calls().iter().map(|c| c.op).collect();
    assert_eq!(ops, [Op::ListStamped]);
}

/// Proves: without leaks the answer is the leaks arm with an empty list and exit class done.
/// Catches an empty sweep reported as a failure or as leaks.
#[test]
fn a_sweep_with_no_leaks_is_done() {
    let mut world = World::new();
    let response = v1::Outcome::from(&world.handle(Request::Leaks));
    assert_eq!(response.exit, ExitClass::Done as i32);
    assert!(
        matches!(&response.result, Some(outcome::Result::Leaks(l)) if l.leaks.is_empty()),
        "{response:?}"
    );
}

/// Proves: an apply through the API that finds a leak answers `done` with the leak in its report
/// message, and a sweep that could not read is in the report as an error and does not change the
/// exit class. Catches a report conversion that drops either field.
#[test]
fn an_apply_report_carries_the_leaks_and_the_sweep_error() {
    let mut world = World::new();
    orphan(&mut world, "ghost");
    let list = list_of(vec![service_of("web", 80)]);
    let desired = lowered(&list);
    let plan = world.plan(&desired, Mode::Apply);
    let hash = plan.hash().clone();
    let done = world.handle(Request::Apply {
        desired,
        plan,
        plan_hash: hash,
        rollback: false,
    });
    let message = v1::Outcome::from(&done);
    assert_eq!(message.exit, ExitClass::Done as i32);
    let Some(outcome::Result::Done(report)) = &message.result else {
        panic!("not done: {message:?}");
    };
    assert_eq!(report.leaks.len(), 1);
    assert_eq!(report.leaks[0].owner, "ghost");
    assert_eq!(report.leak_sweep_failed, None);

    let failed = Report {
        leak_sweep_failed: Some(EngineError::Provider(ProviderError::Transient(
            "down".to_owned(),
        ))),
        ..Report::default()
    };
    let message = v1::Outcome::from(&Outcome::Done(failed));
    assert_eq!(message.exit, ExitClass::Done as i32);
    let Some(outcome::Result::Done(report)) = &message.result else {
        panic!("not done");
    };
    let error = report.leak_sweep_failed.as_ref().expect("the error");
    assert_eq!(error.code, ErrorCode::Provider as i32);
}
