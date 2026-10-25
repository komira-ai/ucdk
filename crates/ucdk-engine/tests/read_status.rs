// SPDX-License-Identifier: Apache-2.0
//! The status request: it shows what the ledger records beside what the provider holds, and it
//! changes nothing. Each scenario runs on the in-memory ledger and on the file ledger.

mod common;

use common::{
    AnyLedger, Backend, FlakyLedger, apply, cell, chain, chain_with, done, lowered, mutations,
    objects, path, physical_of, records,
};
use ucdk_engine::{Engine, EngineError, ObjectStatus, Outcome, Request, Standing, State};
use ucdk_ledger::Ledger as _;
use ucdk_plan::Lowered;
use ucdk_provider::{Kind, Provider as _, ProviderError, Status};
use ucdk_provider_fake::Fake;

const CHAIN: [&str; 4] = ["api/run", "db/run", "web/public", "web/run"];

fn status_of(fake: &mut Fake, ledger: &dyn ucdk_ledger::Ledger, desired: &Lowered) -> Outcome {
    Engine::new(fake, ledger, cell()).handle(Request::Status {
        desired: desired.clone(),
    })
}

fn entries(outcome: Outcome) -> Vec<ObjectStatus> {
    match outcome {
        Outcome::Status(entries) => entries,
        other => panic!("not a status: {other:?}"),
    }
}

fn entry<'a>(list: &'a [ObjectStatus], p: &str) -> &'a ObjectStatus {
    list.iter()
        .find(|e| e.path == path(p))
        .unwrap_or_else(|| panic!("no entry for {p}"))
}

fn paths_of(list: &[ObjectStatus]) -> Vec<String> {
    list.iter().map(|e| e.path.to_string()).collect()
}

fn applied(backend: Backend) -> (Fake, AnyLedger) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    done(apply(&mut fake, &ledger, &chain()));
    (fake, ledger)
}

/// Proves: after an apply, status lists every node of the file in path order, each with a
/// confirmed record, a ready live object and the standing "agrees", and the outputs it shows
/// are the ledger's. Catches a status that reports the plan's steps, or drops outputs.
fn status_after_apply_shows_every_node_confirmed(backend: Backend) {
    let (mut fake, ledger) = applied(backend);
    let list = entries(status_of(&mut fake, &ledger, &chain()));
    assert_eq!(paths_of(&list), CHAIN);
    for e in &list {
        assert_eq!(e.record, Some(State::Confirmed), "{}", e.path);
        assert_eq!(e.live, Some(Status::Ready), "{}", e.path);
        assert_eq!(e.standing, Standing::Agrees, "{}", e.path);
        assert_eq!(e.outputs, records(&ledger)[&e.path].outputs, "{}", e.path);
    }
    assert!(
        list.iter().any(|e| !e.outputs.is_empty()),
        "the fake reports outputs, so some entry shows them"
    );
}

/// Proves: an object deleted out of band shows as missing while its record stays confirmed,
/// and its neighbours still agree. Catches a status that trusts the ledger alone.
fn status_with_an_out_of_band_delete_shows_it_missing(backend: Backend) {
    let (mut fake, ledger) = applied(backend);
    let id = physical_of(&ledger, "db/run");
    fake.delete(&Kind::service(), &id).expect("delete");
    let list = entries(status_of(&mut fake, &ledger, &chain()));
    let gone = entry(&list, "db/run");
    assert_eq!(gone.record, Some(State::Confirmed));
    assert_eq!(gone.live, None);
    assert_eq!(gone.standing, Standing::Missing);
    assert_eq!(entry(&list, "api/run").standing, Standing::Agrees);
    assert_eq!(records(&ledger)[&path("db/run")].state, State::Confirmed);
}

/// Proves: a failed and a converging object are reported as such, with the record still
/// confirmed (status does not decide what a run would do about them). Catches a status that
/// maps every present object to ready.
fn status_reports_failed_and_converging_objects(backend: Backend) {
    let (mut fake, ledger) = applied(backend);
    assert!(fake.set_status(&physical_of(&ledger, "db/run"), Status::Failed));
    assert!(fake.set_status(&physical_of(&ledger, "api/run"), Status::Converging));
    let list = entries(status_of(&mut fake, &ledger, &chain()));
    assert_eq!(entry(&list, "db/run").live, Some(Status::Failed));
    assert_eq!(entry(&list, "api/run").live, Some(Status::Converging));
    assert_eq!(entry(&list, "web/run").live, Some(Status::Ready));
    assert_eq!(entry(&list, "db/run").standing, Standing::Agrees);
}

/// Proves: a run that stopped after its intent shows an intent record beside a live object of
/// ours, as pending. Catches an interrupted run reported as agreeing.
fn status_of_an_interrupted_run_is_pending(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    fake.faults_mut().ambiguous_at_call.replace(1);
    assert!(matches!(
        apply(&mut fake, &ledger, &chain()),
        Outcome::Failed(_)
    ));
    let list = entries(status_of(&mut fake, &ledger, &chain()));
    let e = entry(&list, "db/run");
    assert_eq!(e.record, Some(State::Intent));
    assert_eq!(e.live, Some(Status::Ready));
    assert_eq!(e.standing, Standing::Pending);
    assert_eq!(entry(&list, "api/run").standing, Standing::NotApplied);
}

/// Proves: status performs no ledger write and no provider mutation, in every situation it
/// can meet (confirmed, missing, failed, interrupted). It counts the ledger's writes, the
/// fake's mutating calls, and compares the ledger document and the objects before and after.
/// Catches a status that repairs, reaps or confirms what it finds.
fn status_writes_nothing_and_mutates_nothing(backend: Backend) {
    let (mut fake, inner) = (Fake::standard(), FlakyLedger::on(backend));
    done(apply(&mut fake, &inner, &chain()));
    assert!(fake.set_status(&physical_of(&inner, "web/run"), Status::Failed));
    let id = physical_of(&inner, "db/run");
    fake.delete(&Kind::service(), &id).expect("delete");

    let writes = inner.writes();
    let document = inner.read().expect("read");
    let world = objects(&fake);
    let mutated = mutations(&fake);
    fake.clear_calls();
    let list = entries(status_of(&mut fake, &inner, &chain()));
    assert_eq!(list.len(), 4);
    assert_eq!(inner.writes(), writes, "no ledger write");
    assert_eq!(
        inner.read().expect("read"),
        document,
        "the document is as it was"
    );
    assert_eq!(objects(&fake), world, "the objects are as they were");
    assert!(
        fake.calls().iter().all(|c| !c.op.mutates()),
        "only reads: {:?}",
        fake.calls()
    );
    assert_eq!(mutations(&fake), 0);
    assert!(mutated > 0);

    // The same on an interrupted run's intent.
    let (mut fake, ledger) = (Fake::standard(), FlakyLedger::on(backend));
    fake.faults_mut().ambiguous_at_call.replace(1);
    assert!(matches!(
        apply(&mut fake, &ledger, &chain()),
        Outcome::Failed(_)
    ));
    let writes = ledger.writes();
    let document = ledger.read().expect("read");
    fake.clear_calls();
    entries(status_of(&mut fake, &ledger, &chain()));
    assert_eq!(ledger.writes(), writes);
    assert_eq!(ledger.read().expect("read"), document);
    assert_eq!(mutations(&fake), 0);
}

/// Proves: on an empty ledger every wanted node is "not applied" with no record and no object,
/// a file that names nothing gives an empty list, and a node the file turns off is not listed.
/// Catches a status that lists only recorded objects, or lists unwanted roles.
fn status_on_an_empty_ledger(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    let list = entries(status_of(&mut fake, &ledger, &chain()));
    assert_eq!(paths_of(&list), CHAIN);
    for e in &list {
        assert_eq!((e.record, e.live), (None, None), "{}", e.path);
        assert_eq!(e.standing, Standing::NotApplied);
        assert!(e.outputs.is_empty());
    }
    assert!(entries(status_of(&mut fake, &ledger, &lowered(vec![]))).is_empty());
    let off = entries(status_of(&mut fake, &ledger, &chain_with(5432, false)));
    assert_eq!(paths_of(&off), ["api/run", "db/run", "web/run"]);
}

/// Proves: a recorded object the file no longer names is still listed (status is
/// ledger-driven, not file-driven). Catches a status that forgets what a smaller file left
/// behind.
fn status_lists_recorded_objects_the_file_does_not_name(backend: Backend) {
    let (mut fake, ledger) = applied(backend);
    let list = entries(status_of(&mut fake, &ledger, &lowered(vec![])));
    assert_eq!(paths_of(&list), CHAIN);
    assert!(list.iter().all(|e| e.standing == Standing::Agrees));
}

/// Proves: an object that is not ours is unclaimed: a foreign object under a wanted node's
/// name with no record, and an object of ours whose stamp was stripped under a record.
/// Catches a status that calls any present object "agrees".
fn status_marks_objects_that_are_not_ours_unclaimed(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    fake.add_foreign(&Kind::service(), "db-run", None);
    let list = entries(status_of(&mut fake, &ledger, &chain()));
    let foreign = entry(&list, "db/run");
    assert_eq!(foreign.record, None);
    assert_eq!(foreign.live, Some(Status::Ready));
    assert_eq!(foreign.standing, Standing::Unclaimed);

    let (mut fake, ledger) = applied(backend);
    let id = physical_of(&ledger, "web/public");
    assert!(fake.set_label(&id, "ucdk-owner", None));
    let list = entries(status_of(&mut fake, &ledger, &chain()));
    let stripped = entry(&list, "web/public");
    assert_eq!(stripped.record, Some(State::Confirmed));
    assert_eq!(stripped.standing, Standing::Unclaimed);
}

/// Proves: a ledger that cannot be read, and a provider read that fails, each end the status
/// as a failure that started nothing (no node named, an empty report). Catches a status that
/// swallows a read error into an empty or partial answer.
fn status_fails_when_a_read_fails(backend: Backend) {
    let (mut fake, flaky) = (Fake::standard(), FlakyLedger::on(backend));
    done(apply(&mut fake, &flaky, &chain()));
    flaky.fail_reads(true);
    let Outcome::Failed(f) = status_of(&mut fake, &flaky, &chain()) else {
        panic!("expected a failure");
    };
    assert!(matches!(f.error, EngineError::Ledger(_)));
    assert_eq!((f.at, f.report), (None, ucdk_engine::Report::default()));

    flaky.fail_reads(false);
    fake.faults_mut()
        .fail_reads
        .push(ProviderError::Transient("down".to_owned()));
    let Outcome::Failed(f) = status_of(&mut fake, &flaky, &chain()) else {
        panic!("expected a failure");
    };
    assert_eq!(
        f.error,
        EngineError::Provider(ProviderError::Transient("down".to_owned()))
    );
    assert_eq!(f.at, None);
}

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
    status_after_apply_shows_every_node_confirmed,
    status_with_an_out_of_band_delete_shows_it_missing,
    status_reports_failed_and_converging_objects,
    status_of_an_interrupted_run_is_pending,
    status_writes_nothing_and_mutates_nothing,
    status_on_an_empty_ledger,
    status_lists_recorded_objects_the_file_does_not_name,
    status_marks_objects_that_are_not_ours_unclaimed,
    status_fails_when_a_read_fails,
);
