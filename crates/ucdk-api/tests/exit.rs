// SPDX-License-Identifier: Apache-2.0
//! The exit table: every outcome the engine can answer with has one class, and every class has
//! the code the command line has always exited with.

mod common;

use common::{World, list_of, lowered, path, service_of};
use ucdk_api::v1::ExitClass;
use ucdk_api::{exit_class, failure_class};
use ucdk_engine::{EngineError, Failure, Mode, Outcome, Refusal, Report};
use ucdk_ledger::LedgerError;
use ucdk_plan::{Conflict, ConflictKind, PlanHash};
use ucdk_provider::{ProviderError, Refusal as ProviderRefusal};

fn hash(digit: char) -> PlanHash {
    PlanHash::from_hex(&digit.to_string().repeat(64)).expect("a hash")
}

fn changed() -> Report {
    Report {
        created: vec![path("web/run")],
        ..Report::default()
    }
}

fn failed(report: Report, at: Option<&str>, error: EngineError) -> Failure {
    Failure {
        report,
        at: at.map(path),
        error,
    }
}

#[test]
fn each_class_has_the_exit_code_the_command_line_documents() {
    let table = [
        (ExitClass::Done, 0),
        (ExitClass::Usage, 2),
        (ExitClass::Refused, 3),
        (ExitClass::FailedNoEffect, 4),
        (ExitClass::FailedChanged, 6),
    ];
    for (class, code) in table {
        assert_eq!(class.code(), code, "{class:?}");
        // The wire number is the exit code, so an SDK that reads either gets the same value.
        assert_eq!(class as i32, i32::from(code), "{class:?}");
    }
}

#[test]
fn a_plan_and_a_finished_run_are_done() {
    let mut world = World::new();
    let desired = lowered(&list_of(vec![service_of("web", 80)]));
    let plan = world.plan(&desired, Mode::Apply);
    assert_eq!(exit_class(&Outcome::Planned(plan)), ExitClass::Done);
    assert_eq!(exit_class(&Outcome::Done(changed())), ExitClass::Done);
}

#[test]
fn a_run_that_left_objects_converging_or_failed_is_still_done() {
    let report = Report {
        converging: vec![path("web/run")],
        failed: vec![path("api/run")],
        blocked_by_failed: vec![(path("web/run"), path("api/run"))],
        ..Report::default()
    };
    assert_eq!(exit_class(&Outcome::Done(report)), ExitClass::Done);
}

#[test]
fn every_refusal_is_refused() {
    let refusals = [
        Refusal::Conflicts(vec![Conflict {
            path: path("web/run"),
            why: ConflictKind::Foreign,
        }]),
        Refusal::EmptyScope,
        Refusal::PlanHashMismatch {
            approved: hash('a'),
            plan: hash('b'),
        },
        Refusal::PlanChanged {
            approved: hash('a'),
            now: hash('b'),
        },
        Refusal::UnsupportedStampMode(vec![path("web/run")]),
        Refusal::UnhostedKinds(vec![(path("web/run"), ucdk_provider::Kind::job())]),
    ];
    for refusal in refusals {
        assert_eq!(
            exit_class(&Outcome::Refused(refusal.clone())),
            ExitClass::Refused,
            "{refusal:?}"
        );
    }
}

#[test]
fn a_failure_that_changed_nothing_is_safe_to_repeat() {
    let safe = [
        ProviderError::Refused(ProviderRefusal::Rejected),
        ProviderError::Permanent("no".to_owned()),
        ProviderError::Transient("later".to_owned()),
    ];
    for error in safe {
        let stopped = failed(
            Report::default(),
            Some("web/run"),
            EngineError::Provider(error.clone()),
        );
        assert_eq!(
            failure_class(&stopped),
            ExitClass::FailedNoEffect,
            "{error:?}"
        );
    }
}

#[test]
fn a_failure_before_any_step_started_changed_nothing_whatever_the_error() {
    let stopped = failed(
        Report::default(),
        None,
        EngineError::Ledger(LedgerError::Unavailable("down".to_owned())),
    );
    assert_eq!(failure_class(&stopped), ExitClass::FailedNoEffect);
    let outcome = Outcome::Failed(stopped);
    assert_eq!(exit_class(&outcome), ExitClass::FailedNoEffect);
}

#[test]
fn a_failure_whose_call_may_have_changed_something_needs_a_new_plan() {
    let unsure = [
        EngineError::Provider(ProviderError::InFlight("started".to_owned())),
        EngineError::Provider(ProviderError::Unknown("timed out".to_owned())),
        EngineError::Ledger(LedgerError::Stale),
        EngineError::LedgerBusy,
        EngineError::NotOurs(path("web/run")),
    ];
    for error in unsure {
        let stopped = failed(Report::default(), Some("web/run"), error.clone());
        assert_eq!(
            failure_class(&stopped),
            ExitClass::FailedChanged,
            "{error:?}"
        );
    }
}

#[test]
fn a_failure_after_earlier_steps_changed_things_needs_a_new_plan_even_if_the_error_is_safe() {
    let stopped = failed(
        changed(),
        Some("api/run"),
        EngineError::Provider(ProviderError::Transient("later".to_owned())),
    );
    assert_eq!(failure_class(&stopped), ExitClass::FailedChanged);
    let unstarted = failed(
        changed(),
        None,
        EngineError::Provider(ProviderError::Transient("later".to_owned())),
    );
    assert_eq!(failure_class(&unstarted), ExitClass::FailedChanged);
}
