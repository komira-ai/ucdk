// SPDX-License-Identifier: Apache-2.0
//! Request messages become the engine's requests, or a response that says why they cannot.

mod common;

use common::{World, list_of, lowered, service_of};
use ucdk_api::RequestError;
use ucdk_api::v1::{
    self, ApplyRequest, DestroyRequest, ErrorCode, ExitClass, Mode, PlanRequest, outcome, refusal,
};
use ucdk_engine::{Mode as EngineMode, Outcome, Request};
use ucdk_proto::v1::ResourceList;

fn plan_request(mode: i32, resources: Option<ResourceList>) -> PlanRequest {
    PlanRequest { resources, mode }
}

#[test]
fn a_plan_request_becomes_a_plan_for_the_list_and_the_mode() {
    let list = list_of(vec![service_of("web", 80)]);
    for (mode, want) in [
        (Mode::Apply, EngineMode::Apply),
        (Mode::Destroy, EngineMode::Destroy),
    ] {
        let request = plan_request(mode as i32, Some(list.clone()))
            .to_request()
            .expect("a request");
        let Request::Plan { desired, mode } = request else {
            panic!("not a plan");
        };
        assert_eq!(mode, want);
        assert_eq!(desired.nodes().len(), lowered(&list).nodes().len());
    }
}

#[test]
fn a_plan_request_with_no_list_plans_the_empty_list() {
    let Request::Plan { desired, .. } = plan_request(Mode::Apply as i32, None)
        .to_request()
        .expect("a request")
    else {
        panic!("not a plan");
    };
    assert!(desired.nodes().is_empty());
}

#[test]
fn a_mode_that_is_not_apply_or_destroy_is_invalid() {
    for mode in [Mode::Unspecified as i32, 99] {
        let error = plan_request(mode, None).to_request().unwrap_err();
        assert_eq!(
            error,
            RequestError::Invalid("mode is neither apply nor destroy".to_owned())
        );
    }
}

#[test]
fn a_list_with_findings_is_refused_with_every_finding() {
    let list = list_of(vec![
        service_of("web", 80),
        service_of("web", 81),
        service_of("Bad Id", 80),
    ]);
    let error = plan_request(Mode::Apply as i32, Some(list))
        .to_request()
        .unwrap_err();
    let RequestError::Findings(findings) = &error else {
        panic!("not findings: {error:?}");
    };
    assert!(findings.len() >= 2, "{findings:?}");

    let response = error.clone().into_outcome();
    assert_eq!(response.exit, ExitClass::Refused as i32);
    let Some(outcome::Result::Refused(why)) = response.result else {
        panic!("not refused");
    };
    let Some(refusal::Reason::Findings(wire)) = why.reason else {
        panic!("no findings");
    };
    assert_eq!(wire.findings.len(), findings.len());
    let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
    assert_eq!(why.message, lines.join("; "));
}

#[test]
fn an_invalid_request_answers_usage_with_its_text() {
    let response =
        RequestError::Invalid("mode is neither apply nor destroy".to_owned()).into_outcome();
    assert_eq!(response.exit, ExitClass::Usage as i32);
    let Some(outcome::Result::Invalid(error)) = response.result else {
        panic!("not invalid");
    };
    assert_eq!(error.code, ErrorCode::InvalidRequest as i32);
    assert_eq!(error.message, "mode is neither apply nor destroy");
}

fn hash_text() -> String {
    "ab".repeat(32)
}

#[test]
fn an_apply_request_carries_the_pinned_hash_and_the_rollback_choice() {
    let list = list_of(vec![service_of("web", 80)]);
    let mut world = World::new();
    let plan = world.plan(&lowered(&list), EngineMode::Apply);
    for rollback in [false, true] {
        let approved = ApplyRequest {
            resources: Some(list.clone()),
            plan_hash: hash_text(),
            rollback,
        }
        .approved()
        .expect("approved");
        assert_eq!(approved.plan_hash().as_str(), hash_text());
        let Request::Apply {
            plan_hash,
            rollback: got,
            plan: carried,
            ..
        } = approved.apply(plan.clone())
        else {
            panic!("not an apply");
        };
        assert_eq!((plan_hash.as_str(), got), (hash_text().as_str(), rollback));
        assert_eq!(carried.hash(), plan.hash());
    }
}

#[test]
fn a_destroy_request_carries_the_pinned_hash() {
    let list = list_of(vec![service_of("web", 80)]);
    let mut world = World::new();
    let plan = world.plan(&lowered(&list), EngineMode::Destroy);
    let approved = DestroyRequest {
        resources: Some(list),
        plan_hash: hash_text(),
    }
    .approved()
    .expect("approved");
    let Request::Destroy { plan_hash, .. } = approved.destroy(plan) else {
        panic!("not a destroy");
    };
    assert_eq!(plan_hash.as_str(), hash_text());
}

#[test]
fn a_hash_that_is_not_64_lower_case_hex_digits_is_invalid() {
    let list = list_of(vec![service_of("web", 80)]);
    for bad in ["", "abc", &"AB".repeat(32), &"zz".repeat(32)] {
        let apply = ApplyRequest {
            resources: Some(list.clone()),
            plan_hash: bad.to_owned(),
            rollback: false,
        }
        .approved()
        .unwrap_err();
        let destroy = DestroyRequest {
            resources: Some(list.clone()),
            plan_hash: bad.to_owned(),
        }
        .approved()
        .unwrap_err();
        for error in [apply, destroy] {
            assert!(
                matches!(error, RequestError::Invalid(_)),
                "{bad:?}: {error:?}"
            );
        }
    }
}

#[test]
fn an_approved_run_with_findings_is_refused_before_the_hash_is_read() {
    let list = list_of(vec![service_of("web", 80), service_of("web", 81)]);
    let error = ApplyRequest {
        resources: Some(list),
        plan_hash: "not a hash".to_owned(),
        rollback: false,
    }
    .approved()
    .unwrap_err();
    assert!(matches!(error, RequestError::Findings(_)), "{error:?}");
}

#[test]
fn an_approved_apply_runs_through_the_engine_and_answers_with_a_report() {
    let list = list_of(vec![service_of("web", 80)]);
    let mut world = World::new();
    let desired = lowered(&list);
    let plan = world.plan(&desired, EngineMode::Apply);
    let approved = ApplyRequest {
        resources: Some(list),
        plan_hash: plan.hash().to_string(),
        rollback: false,
    }
    .approved()
    .expect("approved");
    let answered = world.handle(approved.apply(plan));
    assert!(matches!(answered, Outcome::Done(_)), "{answered:?}");
    let wire = v1::Outcome::from(&answered);
    let Some(outcome::Result::Done(report)) = wire.result else {
        panic!("not done");
    };
    assert_eq!(report.created, ["web/run"]);
    assert_eq!(wire.exit, ExitClass::Done as i32);
}
