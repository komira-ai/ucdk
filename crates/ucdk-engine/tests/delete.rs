// SPDX-License-Identifier: Apache-2.0
//! Removal: closed-world delete, what is kept, what is never deleted, reaping, destroy, and
//! rollback.

mod common;

use common::{
    Hooked, apply, apply_with, chain, chain_with, destroy, done, lowered, mutations, objects, path,
    paths, physical_of, plan_of, planned, records, service_of,
};
use ucdk_engine::{Engine, EngineError, Mode, Outcome, Refusal, Request};
use ucdk_ledger::{Change, Ledger as _, MemoryLedger, Origin, State};
use ucdk_plan::{ConflictKind, Retained};
use ucdk_provider::{Kind, ProviderError, Refusal as CallRefusal};
use ucdk_provider_fake::{Fake, FakeConfig, Op};

fn world() -> (Fake, MemoryLedger) {
    (Fake::standard(), MemoryLedger::new())
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
        match Engine::new(&mut fake, &ledger).handle(Request::Plan {
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
    let outcome = Engine::new(&mut fake, &lost).handle(Request::Destroy {
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
        Engine::new(&mut fake, &ledger).handle(Request::Destroy {
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
fn a_reap_against_a_lagging_read_never_drops_a_record_it_can_still_see() {
    // An object whose creation is not yet visible looks gone to the plan. Whatever the lag, a
    // run either reaps (the object really was not there) or stops with the record kept.
    let mut stopped = 0;
    for lag in 1..=14 {
        let (mut fake, ledger) = world();
        *fake.faults_mut().read_lag = lag;
        done(apply(&mut fake, &ledger, &chain()));
        *fake.faults_mut().read_lag = 0;
        match apply(&mut fake, &ledger, &chain_with(5432, false)) {
            Outcome::Failed(f) => {
                assert_eq!(
                    f.error,
                    EngineError::StillPresent(path("web/public")),
                    "lag {lag}"
                );
                assert!(
                    records(&ledger).contains_key(&path("web/public")),
                    "lag {lag}"
                );
                stopped += 1;
            }
            Outcome::Done(_) | Outcome::Refused(_) => {}
            other => panic!("lag {lag}: {other:?}"),
        }
    }
    assert!(
        stopped > 0,
        "some lag makes the object appear between the plan and the reap"
    );
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

#[test]
fn rollback_removes_only_created_this_run() {
    let (mut fake, ledger) = world();
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

#[test]
fn a_failed_run_without_rollback_leaves_what_it_created() {
    let (mut fake, ledger) = world();
    let fail_at = 3;
    fake.faults_mut().fail_at_call.replace(fail_at);
    let f = failure(apply(&mut fake, &ledger, &chain()));
    assert_eq!(f.report.created, paths(&["db/run", "api/run"]));
    assert!(f.report.rolled_back.is_empty());
    assert_eq!(fake.object_count(), 2);
}

#[test]
fn rollback_keeps_what_it_must_keep() {
    let mut fake = keeping();
    let ledger = MemoryLedger::new();
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

#[test]
fn rollback_reports_what_it_could_not_delete_and_goes_on() {
    let (mut fake, ledger) = world();
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
