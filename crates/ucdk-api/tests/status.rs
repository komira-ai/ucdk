// SPDX-License-Identifier: Apache-2.0
//! The status request and response: a list becomes a read-only engine request, the engine's
//! entries become a message that keeps every state apart, and the answer has the exit class
//! the one table gives it.

mod common;

use std::collections::BTreeMap;

use common::{World, list_of, lowered, path, service_of};
use prost::Message as _;
use ucdk_api::v1::{self, ErrorCode, ExitClass, LiveStatus, RecordState, StatusRequest, outcome};
use ucdk_api::{RequestError, exit_class};
use ucdk_engine::{
    EngineError, Failure, Mode, ObjectStatus, Outcome, Report, Request, Standing, State,
};
use ucdk_provider::{Kind, ProviderError, Status};
use ucdk_provider_fake::Op;

/// Proves: a status request carries the list into the engine's read-only request, and a missing
/// list is the empty one. Catches a request that plans or applies instead.
#[test]
fn a_status_request_becomes_the_engine_status_of_the_list() {
    let list = list_of(vec![service_of("web", 80)]);
    let Request::Status { desired } = (StatusRequest {
        resources: Some(list.clone()),
    })
    .to_request()
    .expect("a request") else {
        panic!("not a status");
    };
    assert_eq!(desired.nodes().len(), lowered(&list).nodes().len());
    let Request::Status { desired } = (StatusRequest { resources: None })
        .to_request()
        .expect("a request")
    else {
        panic!("not a status");
    };
    assert!(desired.nodes().is_empty());
}

/// Proves: a list with findings cannot be asked about, and the response is a refusal with exit
/// class refused. Catches a status that looks at the world through a list it could not lower.
#[test]
fn a_list_with_findings_is_refused_for_status_too() {
    let list = list_of(vec![service_of("web", 80), service_of("web", 81)]);
    let error = (StatusRequest {
        resources: Some(list),
    })
    .to_request()
    .unwrap_err();
    let RequestError::Findings(_) = &error else {
        panic!("not findings: {error:?}");
    };
    let response = error.into_outcome();
    assert_eq!(response.exit, ExitClass::Refused as i32);
    assert!(matches!(response.result, Some(outcome::Result::Refused(_))));
}

fn object(state: Option<State>, live: Option<Status>, standing: Standing) -> ObjectStatus {
    ObjectStatus {
        path: path("web/run"),
        kind: Kind::service(),
        record: state,
        live,
        standing,
        outputs: BTreeMap::from([("URL".to_owned(), "the-url".to_owned())]),
    }
}

/// Proves: every record state, live status and standing has its own code on the wire, an absent
/// record or object is absent (not a default), and outputs are kept. Catches two states sharing
/// a code, or "absent" read as the first enum value.
#[test]
fn every_record_state_live_status_and_standing_has_its_own_code() {
    let message = |o: &ObjectStatus| v1::ObjectStatus::from(o);
    for (state, want) in [
        (State::Intent, RecordState::Intent),
        (State::Confirmed, RecordState::Confirmed),
        (State::Reaping, RecordState::Reaping),
    ] {
        let m = message(&object(Some(state), None, Standing::Pending));
        assert_eq!(m.record, Some(want as i32));
    }
    for (status, want) in [
        (Status::Ready, LiveStatus::Ready),
        (Status::Converging, LiveStatus::Converging),
        (Status::Failed, LiveStatus::Failed),
    ] {
        let m = message(&object(None, Some(status), Standing::Unclaimed));
        assert_eq!(m.live, Some(want as i32));
    }
    for (standing, want) in [
        (Standing::Agrees, v1::Standing::Agrees),
        (Standing::Pending, v1::Standing::Pending),
        (Standing::Missing, v1::Standing::Missing),
        (Standing::NotApplied, v1::Standing::NotApplied),
        (Standing::Unclaimed, v1::Standing::Unclaimed),
    ] {
        let m = message(&object(None, None, standing));
        assert_eq!(m.standing, want as i32);
        assert_eq!((m.record, m.live), (None, None));
        assert_eq!(m.path, "web/run");
        assert_eq!(m.kind, Kind::service().as_str());
        assert_eq!(m.outputs["URL"], "the-url");
    }
}

/// Proves: a status through the engine, as bytes, answers with a status response whose entries
/// are the engine's, exit class done, and the same bytes every time; and it makes no mutating
/// provider call. Catches a response that drops entries or reads the exit from the contents.
#[test]
fn a_status_through_bytes_is_a_response_with_exit_class_done() {
    let mut world = World::new();
    let list = list_of(vec![service_of("web", 80)]);
    let desired = lowered(&list);
    let plan = world.plan(&desired, Mode::Apply);
    let hash = plan.hash().clone();
    world.handle(Request::Apply {
        desired: desired.clone(),
        plan,
        plan_hash: hash,
        rollback: false,
    });
    world.fake.clear_calls();

    let request = StatusRequest::decode(
        StatusRequest {
            resources: Some(list),
        }
        .encode_to_vec()
        .as_slice(),
    )
    .expect("decodes");
    let answered = world.handle(request.to_request().expect("a request"));
    let response = v1::Outcome::from(&answered);
    assert_eq!(response.exit, ExitClass::Done as i32);
    let Some(outcome::Result::Status(status)) = &response.result else {
        panic!("not a status: {response:?}");
    };
    assert_eq!(status.objects.len(), 1);
    assert_eq!(status.objects[0].path, "web/run");
    assert_eq!(status.objects[0].standing, v1::Standing::Agrees as i32);
    assert_eq!(
        status.objects[0].record,
        Some(RecordState::Confirmed as i32)
    );
    assert_eq!(status.objects[0].live, Some(LiveStatus::Ready as i32));
    let bytes = response.encode_to_vec();
    assert_eq!(
        v1::Outcome::decode(bytes.as_slice()).expect("decodes"),
        response
    );
    assert_eq!(response.encode_to_vec(), bytes);
    assert!(world.fake.calls().iter().all(|c| !c.op.mutates()));
    assert!(world.fake.calls().iter().any(|c| c.op == Op::Observe));
}

/// Proves: the exit class of a status. Whatever it finds (a missing object, a failed one,
/// nothing at all) is done; a status that could not read stopped before changing anything, so
/// it is "failed, no effect" for every provider error class, even one that for a write would
/// mean "may have changed". Catches a read failure that tells the caller to re-plan.
#[test]
fn the_exit_class_of_a_status_is_done_or_failed_with_no_effect() {
    let findings = vec![
        object(Some(State::Confirmed), None, Standing::Missing),
        object(
            Some(State::Confirmed),
            Some(Status::Failed),
            Standing::Agrees,
        ),
    ];
    assert_eq!(exit_class(&Outcome::Status(findings)), ExitClass::Done);
    assert_eq!(exit_class(&Outcome::Status(Vec::new())), ExitClass::Done);
    for error in [
        EngineError::Provider(ProviderError::Unknown("timed out".to_owned())),
        EngineError::Provider(ProviderError::InFlight("busy".to_owned())),
        EngineError::Provider(ProviderError::Transient("down".to_owned())),
    ] {
        let failed = Outcome::Failed(Failure {
            report: Report::default(),
            at: None,
            error,
        });
        assert_eq!(exit_class(&failed), ExitClass::FailedNoEffect);
        let message = v1::Outcome::from(&failed);
        assert_eq!(message.exit, ExitClass::FailedNoEffect as i32);
        let Some(outcome::Result::Failed(f)) = message.result else {
            panic!("not a failure");
        };
        assert_eq!(f.error.expect("error").code, ErrorCode::Provider as i32);
    }
}
