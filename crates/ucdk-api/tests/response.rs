// SPDX-License-Identifier: Apache-2.0
//! The engine's values as response messages: nothing is dropped, every variant has its own
//! code, and the outcome carries the exit class.

mod common;

use common::{World, list_of, lowered, path, service_of};
use ucdk_api::v1::{
    self, ConflictKind as WireConflict, ErrorCode, ExitClass, FindingKind as WireFinding,
    RetainReason, StepAction, outcome, refusal,
};
use ucdk_engine::{EngineError, Failure, Mode, Outcome, Refusal, Report, Request};
use ucdk_ledger::LedgerError;
use ucdk_plan::{Action, Conflict, ConflictKind, Finding, FindingKind, PlanHash, Retained, Step};
use ucdk_provider::{Kind, PhysicalId, ProviderError};

fn hash(digit: char) -> PlanHash {
    PlanHash::from_hex(&digit.to_string().repeat(64)).expect("a hash")
}

fn step_of(action: Action) -> v1::Step {
    v1::Step::from(&Step {
        path: path("web/run"),
        kind: Kind::new("service"),
        action,
    })
}

#[test]
fn a_plan_keeps_its_steps_in_order_and_its_hash() {
    let mut world = World::new();
    let desired = lowered(&list_of(vec![
        service_of("web", 80),
        service_of("db", 5432),
    ]));
    let plan = world.plan(&desired, Mode::Apply);
    let wire = v1::Plan::from(&plan);
    assert_eq!(wire.hash, plan.hash().as_str());
    let got: Vec<(&str, &str, i32)> = wire
        .steps
        .iter()
        .map(|s| (s.path.as_str(), s.kind.as_str(), s.action))
        .collect();
    let want: Vec<(String, String, i32)> = plan
        .steps()
        .iter()
        .map(|s| {
            (
                s.path.to_string(),
                s.kind.as_str().to_owned(),
                StepAction::Create as i32,
            )
        })
        .collect();
    assert_eq!(wire.steps.len(), 2);
    for (got, want) in got.iter().zip(&want) {
        assert_eq!(
            (got.0, got.1, got.2),
            (want.0.as_str(), want.1.as_str(), want.2)
        );
    }
}

#[test]
fn each_action_has_its_own_code_and_carries_what_it_owns() {
    let plain = [
        (Action::Create, StepAction::Create),
        (Action::Update, StepAction::Update),
        (Action::Noop, StepAction::Noop),
        (Action::Reap, StepAction::Reap),
        (Action::Converging, StepAction::Converging),
        (Action::KnownAfterApply, StepAction::KnownAfterApply),
    ];
    for (action, code) in plain {
        let step = step_of(action.clone());
        assert_eq!(step.action, code as i32, "{action:?}");
        assert_eq!((step.physical_id.as_str(), &step.retained), ("", &None));
        assert_eq!(
            (step.path.as_str(), step.kind.as_str()),
            ("web/run", "service")
        );
    }
    let delete = step_of(Action::Delete {
        physical: PhysicalId::new("phys-1"),
    });
    assert_eq!(delete.action, StepAction::Delete as i32);
    assert_eq!(delete.physical_id, "phys-1");
}

#[test]
fn a_retained_step_says_why() {
    let cases = [
        (Retained::Keep, RetainReason::Keep, ""),
        (
            Retained::Undeletable("in use".to_owned()),
            RetainReason::Undeletable,
            "in use",
        ),
        (Retained::OwnerNotNamed, RetainReason::OwnerNotNamed, ""),
    ];
    for (retained, reason, detail) in cases {
        let step = step_of(Action::Retain(retained.clone()));
        assert_eq!(step.action, StepAction::Retain as i32);
        let why = step.retained.expect("a reason");
        assert_eq!(
            (why.reason, why.detail.as_str()),
            (reason as i32, detail),
            "{retained:?}"
        );
    }
}

#[test]
fn a_report_keeps_every_list_apart() {
    let report = Report {
        created: vec![path("a/run")],
        updated: vec![path("b/run")],
        resumed: vec![path("c/run")],
        deleted: vec![path("d/run")],
        reaped: vec![path("e/run")],
        retained: vec![(path("f/run"), Retained::Keep)],
        converging: vec![path("g/run")],
        failed: vec![path("h/run")],
        blocked_by_failed: vec![(path("i/run"), path("h/run"))],
        rolled_back: vec![path("j/run")],
        rollback_failed: vec![(path("k/run"), EngineError::LedgerBusy)],
    };
    let wire = v1::Report::from(&report);
    assert_eq!(wire.created, ["a/run"]);
    assert_eq!(wire.updated, ["b/run"]);
    assert_eq!(wire.resumed, ["c/run"]);
    assert_eq!(wire.deleted, ["d/run"]);
    assert_eq!(wire.reaped, ["e/run"]);
    assert_eq!(wire.retained.len(), 1);
    assert_eq!(wire.retained[0].path, "f/run");
    assert_eq!(
        wire.retained[0].retained.as_ref().map(|r| r.reason),
        Some(RetainReason::Keep as i32)
    );
    assert_eq!(wire.converging, ["g/run"]);
    assert_eq!(wire.failed, ["h/run"]);
    assert_eq!(
        (
            wire.blocked_by_failed[0].path.as_str(),
            wire.blocked_by_failed[0].producer.as_str()
        ),
        ("i/run", "h/run")
    );
    assert_eq!(wire.rolled_back, ["j/run"]);
    assert_eq!(wire.rollback_failed[0].path, "k/run");
    assert_eq!(
        wire.rollback_failed[0].message,
        EngineError::LedgerBusy.to_string()
    );
}

#[test]
fn an_empty_report_is_the_default_message() {
    assert_eq!(v1::Report::from(&Report::default()), v1::Report::default());
}

fn refusal_of(why: &Refusal) -> v1::Refusal {
    let wire = v1::Refusal::from(why);
    assert_eq!(wire.message, why.to_string());
    wire
}

#[test]
fn a_refusal_names_its_reason_and_keeps_its_text() {
    let wire = refusal_of(&Refusal::EmptyScope);
    assert!(matches!(wire.reason, Some(refusal::Reason::EmptyScope(_))));

    let wire = refusal_of(&Refusal::PlanHashMismatch {
        approved: hash('a'),
        plan: hash('b'),
    });
    let Some(refusal::Reason::PlanHashMismatch(m)) = wire.reason else {
        panic!("wrong reason");
    };
    assert_eq!((m.approved, m.plan), ("a".repeat(64), "b".repeat(64)));

    let wire = refusal_of(&Refusal::PlanChanged {
        approved: hash('a'),
        now: hash('c'),
    });
    let Some(refusal::Reason::PlanChanged(m)) = wire.reason else {
        panic!("wrong reason");
    };
    assert_eq!((m.approved, m.now), ("a".repeat(64), "c".repeat(64)));

    let wire = refusal_of(&Refusal::UnsupportedStampMode(vec![
        path("a/run"),
        path("b/run"),
    ]));
    let Some(refusal::Reason::UnsupportedStampMode(m)) = wire.reason else {
        panic!("wrong reason");
    };
    assert_eq!(m.paths, ["a/run", "b/run"]);

    let wire = refusal_of(&Refusal::UnhostedKinds(vec![(
        path("nightly/run"),
        ucdk_provider::Kind::job(),
    )]));
    let Some(refusal::Reason::UnhostedKinds(m)) = wire.reason else {
        panic!("wrong reason");
    };
    assert_eq!(m.kinds.len(), 1);
    assert_eq!(
        (m.kinds[0].path.as_str(), m.kinds[0].kind.as_str()),
        ("nightly/run", "job")
    );
    assert_eq!(
        wire.message,
        "refused: kind job is not hosted (nightly/run)"
    );
}

#[test]
fn every_conflict_kind_has_its_own_code() {
    let cases = [
        (ConflictKind::Foreign, WireConflict::Foreign, ""),
        (
            ConflictKind::StampWithoutRecord,
            WireConflict::StampWithoutRecord,
            "",
        ),
        (
            ConflictKind::PhysicalIdMismatch,
            WireConflict::PhysicalIdMismatch,
            "",
        ),
        (ConflictKind::StampStripped, WireConflict::StampStripped, ""),
        (ConflictKind::OtherOwner, WireConflict::OtherOwner, ""),
        (
            ConflictKind::OtherTool("tf".to_owned()),
            WireConflict::OtherTool,
            "tf",
        ),
        (
            ConflictKind::VanishedRetained,
            WireConflict::VanishedRetained,
            "",
        ),
        (ConflictKind::RecordedCycle, WireConflict::RecordedCycle, ""),
    ];
    let conflicts: Vec<Conflict> = cases
        .iter()
        .map(|(why, _, _)| Conflict {
            path: path("web/run"),
            why: why.clone(),
        })
        .collect();
    let wire = refusal_of(&Refusal::Conflicts(conflicts.clone()));
    let Some(refusal::Reason::Conflicts(list)) = wire.reason else {
        panic!("wrong reason");
    };
    assert_eq!(list.conflicts.len(), cases.len());
    for ((conflict, wire), (_, kind, marker)) in conflicts.iter().zip(&list.conflicts).zip(&cases) {
        assert_eq!(wire.kind, *kind as i32, "{conflict}");
        assert_eq!(wire.marker, *marker, "{conflict}");
        assert_eq!(wire.path, "web/run");
        assert_eq!(wire.message, conflict.to_string());
    }
}

#[test]
fn every_finding_kind_has_its_own_code() {
    let cases = [
        (FindingKind::BadId, WireFinding::BadId),
        (FindingKind::DuplicateId, WireFinding::DuplicateId),
        (FindingKind::MissingBody, WireFinding::MissingBody),
        (FindingKind::MissingImage, WireFinding::MissingImage),
        (
            FindingKind::Unresolved { what: "parameter" },
            WireFinding::Unresolved,
        ),
        (
            FindingKind::UnsupportedPlatform {
                platform: "linux/arm64".to_owned(),
            },
            WireFinding::UnsupportedPlatform,
        ),
        (FindingKind::EmptyValue, WireFinding::EmptyValue),
        (FindingKind::EnvSecretOverlap, WireFinding::EnvSecretOverlap),
        (
            FindingKind::UnboundRef {
                target: "x".to_owned(),
            },
            WireFinding::UnboundRef,
        ),
        (FindingKind::NoTarget, WireFinding::NoTarget),
        (
            FindingKind::UnsupportedAccess,
            WireFinding::UnsupportedAccess,
        ),
        (FindingKind::AccessToOutput, WireFinding::AccessToOutput),
        (FindingKind::MissingOutput, WireFinding::MissingOutput),
        (FindingKind::NamedOutput, WireFinding::NamedOutput),
        (
            FindingKind::UnexposedOutput {
                target: "x".to_owned(),
                output: "url",
            },
            WireFinding::UnexposedOutput,
        ),
        (
            FindingKind::NotPortable {
                target: "x".to_owned(),
            },
            WireFinding::NotPortable,
        ),
        (
            FindingKind::Cycle {
                members: vec!["a".to_owned(), "b".to_owned()],
            },
            WireFinding::Cycle,
        ),
    ];
    let mut seen = std::collections::BTreeSet::new();
    for (kind, code) in cases {
        let finding = Finding {
            resource: "web".to_owned(),
            field: "service.port".to_owned(),
            kind,
        };
        let wire = v1::Finding::from(&finding);
        assert_eq!(wire.kind, code as i32, "{finding}");
        assert_eq!(
            (wire.resource.as_str(), wire.field.as_str()),
            ("web", "service.port")
        );
        assert_eq!(wire.message, finding.to_string());
        assert!(seen.insert(wire.kind), "two kinds share a code: {finding}");
    }
}

#[test]
fn every_engine_error_has_its_own_code_and_its_text() {
    let cases = [
        (
            EngineError::Provider(ProviderError::Transient("x".to_owned())),
            ErrorCode::Provider,
        ),
        (EngineError::Ledger(LedgerError::Stale), ErrorCode::Ledger),
        (EngineError::LedgerBusy, ErrorCode::LedgerBusy),
        (EngineError::NotOurs(path("a/run")), ErrorCode::NotOurs),
        (
            EngineError::StillPresent(path("a/run")),
            ErrorCode::StillPresent,
        ),
        (
            EngineError::Unwitnessed(path("a/run")),
            ErrorCode::Unwitnessed,
        ),
        (
            EngineError::OutputUnknown {
                path: path("a/run"),
                resource: "db".to_owned(),
                output: "url".to_owned(),
            },
            ErrorCode::OutputUnknown,
        ),
    ];
    for (error, code) in cases {
        let wire = v1::Error::from(&error);
        assert_eq!(wire.code, code as i32, "{error}");
        assert_eq!(wire.message, error.to_string());
    }
}

#[test]
fn a_failure_keeps_its_report_its_place_and_its_error() {
    let failure = Failure {
        report: Report {
            created: vec![path("a/run")],
            ..Report::default()
        },
        at: Some(path("b/run")),
        error: EngineError::LedgerBusy,
    };
    let wire = v1::Failure::from(&failure);
    assert_eq!(wire.report.expect("report").created, ["a/run"]);
    assert_eq!(wire.at.as_deref(), Some("b/run"));
    assert_eq!(
        wire.error.expect("error").code,
        ErrorCode::LedgerBusy as i32
    );

    let unstarted = Failure {
        report: Report::default(),
        at: None,
        error: EngineError::LedgerBusy,
    };
    assert_eq!(v1::Failure::from(&unstarted).at, None);
}

#[test]
fn the_outcome_message_carries_the_result_and_the_exit_class() {
    let mut world = World::new();
    let desired = lowered(&list_of(vec![service_of("web", 80)]));
    let plan = world.plan(&desired, Mode::Apply);

    let planned = v1::Outcome::from(&Outcome::Planned(plan));
    assert!(matches!(planned.result, Some(outcome::Result::Planned(_))));
    assert_eq!(planned.exit, ExitClass::Done as i32);

    let done = v1::Outcome::from(&Outcome::Done(Report::default()));
    assert!(matches!(done.result, Some(outcome::Result::Done(_))));
    assert_eq!(done.exit, ExitClass::Done as i32);

    let refused = v1::Outcome::from(&Outcome::Refused(Refusal::EmptyScope));
    assert!(matches!(refused.result, Some(outcome::Result::Refused(_))));
    assert_eq!(refused.exit, ExitClass::Refused as i32);

    let failed = v1::Outcome::from(&Outcome::Failed(Failure {
        report: Report {
            created: vec![path("a/run")],
            ..Report::default()
        },
        at: Some(path("b/run")),
        error: EngineError::LedgerBusy,
    }));
    assert!(matches!(failed.result, Some(outcome::Result::Failed(_))));
    assert_eq!(failed.exit, ExitClass::FailedChanged as i32);
}

#[test]
fn a_destroy_of_nothing_is_refused_on_the_wire_too() {
    let mut world = World::new();
    let empty = lowered(&list_of(vec![]));
    let answered = world.handle(Request::Plan {
        desired: empty,
        mode: Mode::Destroy,
    });
    let wire = v1::Outcome::from(&answered);
    assert_eq!(wire.exit, ExitClass::Refused as i32);
    let Some(outcome::Result::Refused(why)) = wire.result else {
        panic!("not refused: {answered:?}");
    };
    assert!(matches!(why.reason, Some(refusal::Reason::EmptyScope(_))));
}
