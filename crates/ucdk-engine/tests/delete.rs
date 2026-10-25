// SPDX-License-Identifier: Apache-2.0
//! Removal: closed-world delete, what is kept, what is never deleted, reaping, destroy, and
//! rollback.

mod common;

use common::{
    AnyLedger, Backend, Hooked, apply, apply_with, cell, chain, chain_with, destroy, done, lowered,
    mutations, objects, path, paths, physical_of, plan_of, planned, records, service_of,
};
use ucdk_engine::{Engine, EngineError, Mode, Outcome, Refusal, Request};
use ucdk_ledger::{Change, Ledger as _, MemoryLedger, Origin, State};
use ucdk_plan::{ConflictKind, Retained};
use ucdk_provider::{Kind, ProviderError, Refusal as CallRefusal};
use ucdk_provider_fake::{Fake, FakeConfig, Op};

fn world() -> (Fake, MemoryLedger) {
    (Fake::standard(), MemoryLedger::new())
}

fn world_on(backend: Backend) -> (Fake, AnyLedger) {
    (Fake::standard(), backend.ledger())
}

fn deletes(fake: &Fake) -> usize {
    fake.calls().iter().filter(|c| c.op == Op::Delete).count()
}

fn conflicts(outcome: Outcome) -> Vec<(String, ConflictKind)> {
    let Outcome::Refused(Refusal::Conflicts(list)) = outcome else {
        panic!("expected a refusal for conflicts, got {outcome:?}");
    };
    list.into_iter()
        .map(|c| (c.path.to_string(), c.why))
        .collect()
}

fn failure(outcome: Outcome) -> ucdk_engine::Failure {
    match outcome {
        Outcome::Failed(f) => f,
        other => panic!("the run should have failed: {other:?}"),
    }
}

/// A fake that keeps `public` objects as data.
fn keeping() -> Fake {
    Fake::new(FakeConfig {
        stateful: [Kind::new("public")].into(),
        ..FakeConfig::standard()
    })
}

#[test]
fn removed_node_deleted_only_if_stamp_and_record() {
    // Stamp and record: the object goes, and so does its record.
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let report = done(apply(&mut fake, &ledger, &chain_with(5432, false)));
    assert_eq!(report.deleted, paths(&["web/public"]));
    assert!(!records(&ledger).contains_key(&path("web/public")));
    assert_eq!(fake.object_count(), 3);

    // A record but no stamp: the run is refused whole and nothing is deleted.
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "web/public");
    assert!(fake.set_label(&id, "ucdk-owner", None));
    let before = mutations(&fake);
    let outcome = apply(&mut fake, &ledger, &chain_with(5432, false));
    assert_eq!(
        conflicts(outcome),
        [("web/public".to_owned(), ConflictKind::StampStripped)]
    );
    assert_eq!(mutations(&fake), before);

    // A stamp but no record: refused whole, nothing deleted.
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let before = mutations(&fake);
    let outcome = apply(&mut fake, &MemoryLedger::new(), &chain_with(5432, false));
    let found = conflicts(outcome);
    assert!(
        found
            .iter()
            .all(|(_, why)| *why == ConflictKind::StampWithoutRecord)
    );
    assert_eq!(found.len(), 4);
    assert_eq!(mutations(&fake), before);
}

#[test]
fn destroy_with_ledger_lost_deletes_nothing() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let approved = planned(
        match Engine::new(&mut fake, &ledger, cell()).handle(Request::Plan {
            desired: chain(),
            mode: Mode::Destroy,
        }) {
            out @ Outcome::Planned(_) => out,
            other => panic!("{other:?}"),
        },
    );
    let hash = approved.hash().clone();
    // The ledger is lost: the objects carry our stamp and nothing records them.
    let lost = MemoryLedger::new();
    let outcome = Engine::new(&mut fake, &lost, cell()).handle(Request::Destroy {
        desired: chain(),
        plan: approved,
        plan_hash: hash,
    });
    let found = conflicts(outcome);
    assert_eq!(found.len(), 4);
    assert!(
        found
            .iter()
            .all(|(_, why)| *why == ConflictKind::StampWithoutRecord)
    );
    assert_eq!(deletes(&fake), 0);
    assert_eq!(fake.object_count(), 4);
}

#[test]
fn destroy_deletes_dependents_before_what_they_need() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let report = done(destroy(&mut fake, &ledger, &chain()));
    let at = |p: &str| {
        report
            .deleted
            .iter()
            .position(|d| d.as_str() == p)
            .unwrap_or_else(|| panic!("{p} was not deleted: {:?}", report.deleted))
    };
    assert_eq!(report.deleted.len(), 4);
    assert!(at("web/public") < at("web/run"));
    assert!(at("web/run") < at("api/run"));
    assert!(at("api/run") < at("db/run"));
    assert_eq!(fake.object_count(), 0);
    assert!(records(&ledger).is_empty());
}

#[test]
fn destroy_leaves_an_owner_the_file_does_not_name() {
    let (mut fake, ledger) = world();
    let both = lowered(vec![
        service_of("app", 80, false, &[]),
        service_of("other", 81, false, &[]),
    ]);
    done(apply(&mut fake, &ledger, &both));
    let only_app = lowered(vec![service_of("app", 80, false, &[])]);
    let report = done(destroy(&mut fake, &ledger, &only_app));
    assert_eq!(report.deleted, paths(&["app/run"]));
    assert_eq!(
        report.retained,
        [(path("other/run"), Retained::OwnerNotNamed)]
    );
    assert_eq!(fake.object_count(), 1);
}

#[test]
fn destroy_of_a_file_that_names_nothing_is_refused() {
    let (mut fake, ledger) = world();
    let none = lowered(Vec::new());
    let approved = planned(plan_of(&mut fake, &ledger, &none));
    let hash = approved.hash().clone();
    for outcome in [
        destroy(&mut fake, &ledger, &none),
        Engine::new(&mut fake, &ledger, cell()).handle(Request::Destroy {
            desired: none.clone(),
            plan: approved,
            plan_hash: hash,
        }),
    ] {
        let Outcome::Refused(refusal) = outcome else {
            panic!("expected a refusal");
        };
        assert_eq!(refusal, Refusal::EmptyScope);
        assert_eq!(
            refusal.to_string(),
            "refused: the file names nothing to destroy"
        );
    }
}

#[test]
fn keep_node_retained_on_removal() {
    let mut fake = keeping();
    let ledger = MemoryLedger::new();
    done(apply(&mut fake, &ledger, &chain()));
    for _ in 0..2 {
        let report = done(apply(&mut fake, &ledger, &chain_with(5432, false)));
        assert_eq!(report.retained, [(path("web/public"), Retained::Keep)]);
        assert!(report.deleted.is_empty());
    }
    assert_eq!(fake.object_count(), 4, "the data is kept");
    assert_eq!(
        records(&ledger)[&path("web/public")].state,
        State::Confirmed
    );
    assert_eq!(deletes(&fake), 0);
    // Destroy keeps it too.
    let report = done(destroy(&mut fake, &ledger, &chain()));
    assert!(
        report
            .retained
            .iter()
            .any(|(p, why)| p.as_str() == "web/public" && *why == Retained::Keep)
    );
    assert_eq!(deletes(&fake), 3);
    assert_eq!(fake.object_count(), 1);
}

#[test]
fn undeletable_never_lifted() {
    let mut fake = Fake::new(FakeConfig {
        deletion_forbidden: [Kind::new("public")].into(),
        ..FakeConfig::standard()
    });
    let ledger = MemoryLedger::new();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "web/public");
    assert!(fake.set_unmodelled(&id, "data", "x"));
    let undeletable = |report: &ucdk_engine::Report| {
        report
            .retained
            .iter()
            .filter(|(p, why)| {
                p.as_str() == "web/public" && matches!(why, Retained::Undeletable(_))
            })
            .count()
    };
    for _ in 0..2 {
        let report = done(apply(&mut fake, &ledger, &chain_with(5432, false)));
        assert_eq!(undeletable(&report), 1);
    }
    let report = done(destroy(&mut fake, &ledger, &chain()));
    assert_eq!(undeletable(&report), 1);
    assert_eq!(deletes(&fake), 3, "only the three deletable objects went");
    assert_eq!(fake.object_count(), 1);
}

#[test]
fn adopted_origin_is_never_deleted() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let mut record = records(&ledger)[&path("web/public")].clone();
    record.origin = Origin::Adopted;
    let token = ledger.read().expect("read").token;
    ledger.write(&token, Change::Put(record)).expect("write");
    let report = done(apply(&mut fake, &ledger, &chain_with(5432, false)));
    assert_eq!(report.retained.len(), 1);
    assert!(report.deleted.is_empty());
    assert_eq!(deletes(&fake), 0);
    assert_eq!(fake.object_count(), 4);
}

#[test]
fn transient_read_does_not_reap() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    // The object disappears from the cloud; the file no longer wants its node.
    let id = physical_of(&ledger, "web/public");
    ucdk_provider::Provider::delete(&mut fake, &Kind::new("public"), &id).expect("delete");
    let wanted = chain_with(5432, false);

    // The read that would show it gone fails: the record stays.
    fake.faults_mut()
        .fail_reads
        .push(ProviderError::Transient("busy".to_owned()));
    let f = failure(apply(&mut fake, &ledger, &wanted));
    assert_eq!(f.at, None);
    assert!(records(&ledger).contains_key(&path("web/public")));

    // The same failure at the read made just before the reap: the record still stays.
    let reads_in_plan = wanted.nodes().len();
    let provider_calls = 2 * reads_in_plan + 1; // the driver plans, then the run plans again
    let mut hooked = Hooked::new(&mut fake, provider_calls, |fake| {
        fake.faults_mut()
            .fail_reads
            .push(ProviderError::Transient("busy".to_owned()));
    });
    let f = failure(apply(&mut hooked, &ledger, &wanted));
    drop(hooked);
    assert_eq!(f.at, Some(path("web/public")));
    assert!(matches!(
        f.error,
        EngineError::Provider(ProviderError::Transient(_))
    ));
    assert!(records(&ledger).contains_key(&path("web/public")));

    // A read that shows it gone reaps the record.
    let report = done(apply(&mut fake, &ledger, &wanted));
    assert_eq!(report.reaped, paths(&["web/public"]));
    assert!(!records(&ledger).contains_key(&path("web/public")));
}

#[test]
fn a_delete_whose_object_is_still_read_is_not_reaped() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    *fake.faults_mut().read_lag = 3;
    let f = failure(apply(&mut fake, &ledger, &chain_with(5432, false)));
    assert_eq!(f.at, Some(path("web/public")));
    assert_eq!(f.error, EngineError::StillPresent(path("web/public")));
    assert_eq!(
        records(&ledger)[&path("web/public")].state,
        State::Reaping,
        "the record is left where the delete began"
    );
    // Once reads tell the truth the next run finds it gone and reaps the record.
    *fake.faults_mut().read_lag = 0;
    let report = done(apply(&mut fake, &ledger, &chain_with(5432, false)));
    assert_eq!(report.reaped, paths(&["web/public"]));
    assert!(!records(&ledger).contains_key(&path("web/public")));
}

#[test]
fn a_reap_against_a_lagging_read_never_orphans_an_object() {
    // The create is not readable for `lag` reads. Whatever the lag, a run either stops with the
    // record kept or finishes with no object left that has no record.
    for lag in (0..=14).chain([1000]) {
        let (mut fake, ledger) = world();
        *fake.faults_mut().read_lag = lag;
        done(apply(&mut fake, &ledger, &chain()));
        *fake.faults_mut().read_lag = 0;
        match apply(&mut fake, &ledger, &chain_with(5432, false)) {
            Outcome::Done(_) | Outcome::Failed(_) | Outcome::Refused(_) => {}
            other => panic!("lag {lag}: {other:?}"),
        }
        assert!(
            fake.object_count() <= records(&ledger).len(),
            "lag {lag}: an object has no record"
        );
    }
}

#[test]
fn an_object_never_read_back_after_its_create_is_not_reaped_when_a_read_shows_none() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain_with(5432, false)));
    // web/public is created now, but nothing reads it for a long while.
    *fake.faults_mut().read_lag = 1000;
    done(apply(&mut fake, &ledger, &chain()));
    *fake.faults_mut().read_lag = 0;
    let objects_before = fake.object_count();

    let f = failure(apply(&mut fake, &ledger, &chain_with(5432, false)));
    assert_eq!(f.error, EngineError::Unwitnessed(path("web/public")));
    assert!(records(&ledger).contains_key(&path("web/public")));
    assert_eq!(fake.object_count(), objects_before);
}

#[test]
fn an_object_read_back_after_its_create_is_reaped_when_it_is_gone() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(
        records(&ledger)[&path("web/public")].state,
        State::Confirmed
    );
    let id = physical_of(&ledger, "web/public");
    ucdk_provider::Provider::delete(&mut fake, &Kind::new("public"), &id).expect("delete");
    let report = done(apply(&mut fake, &ledger, &chain_with(5432, false)));
    assert_eq!(report.reaped, paths(&["web/public"]));
    assert!(!records(&ledger).contains_key(&path("web/public")));
}

#[test]
fn an_object_that_lost_its_stamp_after_the_plan_is_not_deleted() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "web/public");
    let wanted = chain_with(5432, false);
    // The driver's plan and the run's own plan read first; the stamp goes just before the delete step's own read.
    let at = 2 * wanted.nodes().len() + 1;
    let mut hooked = Hooked::new(&mut fake, at, move |fake| {
        assert!(fake.set_label(&id, "ucdk-owner", None));
    });
    let f = failure(apply(&mut hooked, &ledger, &wanted));
    drop(hooked);
    assert_eq!(f.error, EngineError::NotOurs(path("web/public")));
    assert_eq!(deletes(&fake), 0);
}

#[test]
fn a_delete_the_provider_refuses_leaves_the_record_reaping_and_the_next_run_deletes() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let next = mutations(&fake) as u64 + 1;
    fake.faults_mut().fail_at_call.replace(next);
    let f = failure(apply(&mut fake, &ledger, &chain_with(5432, false)));
    assert_eq!(
        f.error,
        EngineError::Provider(ProviderError::Refused(CallRefusal::Rejected))
    );
    assert_eq!(records(&ledger)[&path("web/public")].state, State::Reaping);
    let report = done(apply(&mut fake, &ledger, &chain_with(5432, false)));
    assert_eq!(report.deleted, paths(&["web/public"]));
    assert!(!records(&ledger).contains_key(&path("web/public")));
}

fn rollback_removes_only_created_this_run(backend: Backend) {
    let (mut fake, ledger) = world_on(backend);
    let base = lowered(vec![
        service_of("api", 81, false, &["db"]),
        service_of("db", 5432, false, &[]),
    ]);
    done(apply(&mut fake, &ledger, &base));
    // The next run changes `db`, then creates `web/run` and `web/public`; the last create is
    // refused, so the run fails after one update and one creation.
    let grown = lowered(vec![
        service_of("web", 80, true, &["api"]),
        service_of("api", 81, false, &["db"]),
        service_of("db", 6000, false, &[]),
    ]);
    let fail_at = mutations(&fake) as u64 + 3;
    fake.faults_mut().fail_at_call.replace(fail_at);
    let f = failure(apply_with(&mut fake, &ledger, &grown, true));
    assert_eq!(f.at, Some(path("web/public")));
    assert_eq!(f.report.created, paths(&["web/run"]));
    assert_eq!(f.report.updated, paths(&["db/run"]));
    assert_eq!(f.report.rolled_back, paths(&["web/run"]));
    assert!(f.report.rollback_failed.is_empty());
    // What the run created is gone, with its record; what it updated and what was there stay.
    assert_eq!(fake.object_count(), 2);
    assert!(
        objects(&fake).contains("port=6000"),
        "the update is not reverted"
    );
    let kept: Vec<String> = records(&ledger).keys().map(ToString::to_string).collect();
    assert_eq!(kept, ["api/run", "db/run"]);
}

fn a_failed_run_without_rollback_leaves_what_it_created(backend: Backend) {
    let (mut fake, ledger) = world_on(backend);
    let fail_at = 3;
    fake.faults_mut().fail_at_call.replace(fail_at);
    let f = failure(apply(&mut fake, &ledger, &chain()));
    assert_eq!(f.report.created, paths(&["db/run", "api/run"]));
    assert!(f.report.rolled_back.is_empty());
    assert_eq!(fake.object_count(), 2);
}

fn rollback_keeps_what_it_must_keep(backend: Backend) {
    let mut fake = keeping();
    let ledger = backend.ledger();
    // The fifth create, `zzz/run`, is refused; the four before it are rolled back, except the
    // kept kind.
    let more = lowered(vec![
        service_of("web", 80, true, &["api"]),
        service_of("api", 81, false, &["db"]),
        service_of("db", 5432, false, &[]),
        service_of("zzz", 82, false, &[]),
    ]);
    fake.faults_mut().fail_at_call.replace(5);
    let f = failure(apply_with(&mut fake, &ledger, &more, true));
    assert_eq!(f.at, Some(path("zzz/run")));
    assert_eq!(
        f.report.rolled_back,
        paths(&["web/run", "api/run", "db/run"])
    );
    assert_eq!(f.report.retained, [(path("web/public"), Retained::Keep)]);
    assert_eq!(fake.object_count(), 1, "the kept object stays");
}

fn rollback_reports_what_it_could_not_delete_and_goes_on(backend: Backend) {
    let (mut fake, ledger) = world_on(backend);
    // Creates: `db/run`, `api/run`; the third is refused. The rollback's first delete
    // (mutation 4, of `api/run`) times out; the second (of `db/run`) succeeds.
    {
        let faults = fake.faults_mut();
        faults.fail_at_call.replace(3);
        faults.ambiguous_at_call.replace(4);
    }
    let f = failure(apply_with(&mut fake, &ledger, &chain(), true));
    assert_eq!(f.report.rolled_back, paths(&["db/run"]));
    assert_eq!(f.report.rollback_failed.len(), 1);
    assert_eq!(f.report.rollback_failed[0].0, path("api/run"));
    assert!(matches!(
        f.report.rollback_failed[0].1,
        EngineError::Provider(ProviderError::Unknown(_))
    ));
}

#[test]
fn data_that_appears_after_the_plan_stops_the_delete() {
    let mut fake = Fake::new(FakeConfig {
        deletion_forbidden: [Kind::new("public")].into(),
        ..FakeConfig::standard()
    });
    let ledger = MemoryLedger::new();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "web/public");
    let wanted = chain_with(5432, false);
    // The object holds nothing the catalog lacks when the plan reads it, and does by the time
    // the delete step reads it again.
    let at = 2 * wanted.nodes().len() + 1;
    let mut hooked = Hooked::new(&mut fake, at, move |fake| {
        assert!(fake.set_unmodelled(&id, "data", "x"));
    });
    let report = done(apply(&mut hooked, &ledger, &wanted));
    drop(hooked);
    assert!(report.deleted.is_empty());
    assert!(matches!(
        report.retained[..],
        [(_, Retained::Undeletable(_))]
    ));
    assert_eq!(deletes(&fake), 0);
}

#[test]
fn a_record_dropped_by_another_run_before_the_reap_is_not_an_error() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "web/public");
    ucdk_provider::Provider::delete(&mut fake, &Kind::new("public"), &id).expect("delete");
    let wanted = chain_with(5432, false);
    // The plan reads the record and finds the object gone; another run reaps the record before
    // this run's reap step looks at it (the hook runs at the plan's last read).
    let at = 2 * wanted.nodes().len();
    let peer = ledger.clone();
    let mut hooked = Hooked::new(&mut fake, at, move |_| {
        let token = peer.read().expect("read").token;
        peer.write(&token, Change::Remove(path("web/public")))
            .expect("write");
    });
    let report = done(apply(&mut hooked, &ledger, &wanted));
    drop(hooked);
    assert!(report.reaped.is_empty());
    assert!(!records(&ledger).contains_key(&path("web/public")));
}

#[test]
fn a_read_that_fails_as_the_run_begins_stops_it_before_any_change() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let wanted = chain_with(5432, false);
    let mutated = mutations(&fake);
    // The driver's plan reads first; the run's own first read fails.
    let at = wanted.nodes().len() + 1;
    let mut hooked = Hooked::new(&mut fake, at, |fake| {
        fake.faults_mut()
            .fail_reads
            .push(ProviderError::Transient("busy".to_owned()));
    });
    let f = failure(apply(&mut hooked, &ledger, &wanted));
    drop(hooked);
    assert_eq!(f.at, None);
    assert_eq!(mutations(&fake), mutated);
}

#[test]
fn rollback_leaves_the_record_of_an_object_it_cannot_read() {
    let (mut fake, ledger) = world();
    // The creates are not readable; the third is refused, so the rollback looks for the first two.
    *fake.faults_mut().read_lag = 1000;
    fake.faults_mut().fail_at_call.replace(3);
    let f = failure(apply_with(&mut fake, &ledger, &chain(), true));
    *fake.faults_mut().read_lag = 0;
    assert!(f.report.rolled_back.is_empty());
    assert_eq!(
        f.report.rollback_failed,
        [
            (path("api/run"), EngineError::Unwitnessed(path("api/run"))),
            (path("db/run"), EngineError::Unwitnessed(path("db/run"))),
        ]
    );
    assert_eq!(fake.object_count(), 2);
    assert!(records(&ledger).contains_key(&path("db/run")));
}

#[test]
fn a_failed_read_back_after_a_create_keeps_an_intent_that_names_the_object() {
    let (mut fake, ledger) = world();
    // Two plans read the four nodes, then the first step reads, creates and reads back.
    let at = 2 * chain().nodes().len() + 3;
    let mut hooked = Hooked::new(&mut fake, at, |fake| {
        fake.faults_mut()
            .fail_reads
            .push(ProviderError::Transient("busy".to_owned()));
    });
    let f = failure(apply(&mut hooked, &ledger, &chain()));
    drop(hooked);
    assert_eq!(f.at, Some(path("db/run")));
    assert!(matches!(
        f.error,
        EngineError::Provider(ProviderError::Transient(_))
    ));
    assert_eq!(f.report.created, paths(&["db/run"]));
    let record = &records(&ledger)[&path("db/run")];
    assert_eq!(record.state, State::Intent);
    assert!(record.physical.is_some());
    // The next run reads the object, confirms the record, and finishes the rest.
    done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(records(&ledger)[&path("db/run")].state, State::Confirmed);
    assert_eq!(fake.object_count(), 4);
}

#[test]
fn an_object_that_reappears_before_the_reap_keeps_its_record() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let record = records(&ledger)[&path("web/public")].clone();
    let id = physical_of(&ledger, "web/public");
    ucdk_provider::Provider::delete(&mut fake, &Kind::new("public"), &id).expect("delete");
    // The record names no object, so the object is found by its name.
    let token = ledger.read().expect("read").token;
    let nameless = ucdk_ledger::Record {
        physical: None,
        ..record.clone()
    };
    ledger.write(&token, Change::Put(nameless)).expect("write");
    let wanted = chain_with(5432, false);
    // Both plans find the object gone; another client makes it again just before the reap reads.
    let at = 2 * wanted.nodes().len() + 1;
    let again = ucdk_provider::Node::new(
        path("web/public"),
        record.kind.clone(),
        record.name.clone(),
        Vec::new(),
    )
    .expect("a node");
    let mut hooked = Hooked::new(&mut fake, at, move |fake| {
        let stamp = ucdk_provider::Stamp::for_node(&cell(), &path("web/public"));
        ucdk_provider::Provider::create(fake, &again, &stamp).expect("create");
    });
    let f = failure(apply(&mut hooked, &ledger, &wanted));
    drop(hooked);
    assert_eq!(f.error, EngineError::StillPresent(path("web/public")));
    assert!(records(&ledger).contains_key(&path("web/public")));
}

/// An unwanted `web/public` whose record is an intent that names no object (a create whose
/// reply was lost), with no object of that name left in the world.
fn dangling_intent() -> (Fake, MemoryLedger) {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let record = records(&ledger)[&path("web/public")].clone();
    let id = physical_of(&ledger, "web/public");
    ucdk_provider::Provider::delete(&mut fake, &Kind::new("public"), &id).expect("delete");
    let token = ledger.read().expect("read").token;
    let nameless = ucdk_ledger::Record {
        state: State::Intent,
        physical: None,
        ..record
    };
    ledger.write(&token, Change::Put(nameless)).expect("write");
    (fake, ledger)
}

/// The reads of one run that plans twice before it acts: each plan reads the six nodes the
/// records and the file name, so the step's own first read is the next one.
const READS_BEFORE_THE_STEP: usize = 12;

fn reads_of(fake: &Fake, op: Op) -> usize {
    fake.calls().iter().filter(|c| c.op == op).count()
}

#[test]
fn an_unwanted_intent_whose_object_is_definitively_gone_is_reaped() {
    let (mut fake, ledger) = dangling_intent();
    let before = mutations(&fake);
    let report = done(apply(&mut fake, &ledger, &chain_with(5432, false)));
    assert_eq!(report.reaped, paths(&["web/public"]));
    assert!(!records(&ledger).contains_key(&path("web/public")));
    assert_eq!(
        mutations(&fake),
        before,
        "no provider call changed anything"
    );
    // The record went only after the owner's objects were listed, and the object read by name,
    // more than once.
    assert!(reads_of(&fake, Op::ListOwned) >= 2);
    // Every other record is untouched, and a destroy then takes the rest.
    assert_eq!(fake.object_count(), 3);
    done(destroy(&mut fake, &ledger, &chain()));
    assert_eq!(fake.object_count(), 0);
    assert!(records(&ledger).is_empty());
}

#[test]
fn an_unwanted_intent_is_not_reaped_on_a_lagging_read() {
    // The object the lost reply made becomes readable only after `lag` reads. Whichever of the
    // step's reads is the first to show it, the record stays and the run stops.
    for lag in 1..=4 {
        let (mut fake, ledger) = dangling_intent();
        let before = mutations(&fake);
        let again = ucdk_provider::Node::new(
            path("web/public"),
            Kind::new("public"),
            "web-public",
            Vec::new(),
        )
        .expect("a node");
        let mut hooked = Hooked::new(&mut fake, READS_BEFORE_THE_STEP + 1, move |fake| {
            let stamp = ucdk_provider::Stamp::for_node(&cell(), &path("web/public"));
            *fake.faults_mut().read_lag = lag;
            ucdk_provider::Provider::create(fake, &again, &stamp).expect("create");
            *fake.faults_mut().read_lag = 0;
        });
        let f = failure(apply(&mut hooked, &ledger, &chain_with(5432, false)));
        drop(hooked);
        assert_eq!(
            f.error,
            EngineError::StillPresent(path("web/public")),
            "lag {lag}"
        );
        let record = &records(&ledger)[&path("web/public")];
        assert_eq!(record.state, State::Intent, "lag {lag}");
        assert_eq!(record.physical, None, "lag {lag}");
        // Only the hook's create changed the world.
        assert_eq!(mutations(&fake), before + 1, "lag {lag}");
        // Once the object reads, the next run takes it for the record and a destroy removes it.
        done(apply(&mut fake, &ledger, &chain_with(5432, false)));
        done(destroy(&mut fake, &ledger, &chain()));
        assert_eq!(fake.object_count(), 0, "lag {lag}");
    }
}

#[test]
fn an_unwanted_intent_is_not_reaped_on_a_transient_read() {
    // Each read of the step in turn fails in each way that is not an answer.
    let errors = [
        ProviderError::Transient("busy".to_owned()),
        ProviderError::InFlight("working".to_owned()),
        ProviderError::Unknown("timed out".to_owned()),
    ];
    for error in errors {
        for read in 1..=5 {
            let (mut fake, ledger) = dangling_intent();
            let before = mutations(&fake);
            let injected = error.clone();
            let mut hooked = Hooked::new(&mut fake, READS_BEFORE_THE_STEP + read, move |fake| {
                fake.faults_mut().fail_reads.push(injected);
            });
            let f = failure(apply(&mut hooked, &ledger, &chain_with(5432, false)));
            drop(hooked);
            assert_eq!(
                f.error,
                EngineError::Provider(error.clone()),
                "{error:?} {read}"
            );
            assert_eq!(f.at, Some(path("web/public")), "{error:?} {read}");
            assert_eq!(records(&ledger)[&path("web/public")].state, State::Intent);
            assert_eq!(mutations(&fake), before, "{error:?} {read}");
        }
    }
}

#[test]
fn a_lost_create_reply_and_a_lagging_read_do_not_let_destroy_orphan_the_object() {
    let (mut fake, ledger) = world();
    // The create lands and its reply is lost: the intent names no object. Reads then lag, so
    // the object reads as absent through the destroy's two plans (six reads each) and the
    // step's first read, and shows on its second.
    fake.faults_mut().ambiguous_at_call.replace(1);
    *fake.faults_mut().read_lag = 13;
    let f = failure(apply(&mut fake, &ledger, &chain()));
    assert_eq!(f.at, Some(path("db/run")));
    assert_eq!(fake.object_count(), 1);

    // A destroy does not drop the record on that read: the object stays recorded.
    let f = failure(destroy(&mut fake, &ledger, &chain()));
    assert_eq!(f.error, EngineError::StillPresent(path("db/run")));
    assert_eq!(fake.object_count(), 1);
    assert_eq!(records(&ledger)[&path("db/run")].state, State::Intent);

    // Once the lag has passed, an apply takes the object and a destroy removes it.
    for _ in 0..30 {
        plan_of(&mut fake, &ledger, &chain());
    }
    *fake.faults_mut().read_lag = 0;
    *fake.faults_mut().ambiguous_at_call = None;
    done(apply(&mut fake, &ledger, &chain()));
    done(destroy(&mut fake, &ledger, &chain()));
    assert_eq!(fake.object_count(), 0);
    assert!(records(&ledger).is_empty());
}

// ---- rollback, once per ledger ---------------------------------------------------------------

/// Runs each named scenario on the in-memory ledger and on the file ledger, as the tests
/// `<name>::memory` and `<name>::file`.
macro_rules! on_every_ledger {
    ($($name:ident),* $(,)?) => {
        $(
            mod $name {
                #[test]
                fn memory() {
                    super::$name(super::Backend::Memory);
                }

                #[test]
                fn file() {
                    super::$name(super::Backend::File);
                }
            }
        )*
    };
}

on_every_ledger!(
    rollback_removes_only_created_this_run,
    a_failed_run_without_rollback_leaves_what_it_created,
    rollback_keeps_what_it_must_keep,
    rollback_reports_what_it_could_not_delete_and_goes_on,
);
