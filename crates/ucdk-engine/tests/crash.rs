// SPDX-License-Identifier: Apache-2.0
//! Crash safety and races: a run that stops at any call, with the call failing or its reply
//! lost, is followed by a run that leaves the world exactly as a clean run would; and a run
//! that takes over from a stalled one creates nothing twice.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::{
    AnyLedger, Backend, FlakyLedger, Hooked, apply, apply_with, chain, destroy, done, lowered,
    mutations, objects, owned_by_fake, path, paths, physical_of, records, service_of,
    service_reading,
};
use ucdk_engine::{DEFAULT_LEASE_TTL, EngineError, Outcome, Refusal, Report};
use ucdk_ledger::{
    Change, Fence, Ledger as _, LedgerError, MAX_CLOCK_SKEW, ManualClock, Record, State,
};
use ucdk_plan::{ConflictKind, Lowered};
use ucdk_proto::v1::Output;
use ucdk_provider::{Kind, Provider as _, ProviderError, Refusal as CallRefusal};
use ucdk_provider_fake::{Fake, Op};

fn failure(outcome: Outcome) -> ucdk_engine::Failure {
    match outcome {
        Outcome::Failed(f) => f,
        other => panic!("the run should have failed: {other:?}"),
    }
}

// ---- one crashed create ----------------------------------------------------------------------

fn crash_after_intent_adopts_on_rerun(backend: Backend) {
    let mut fake = Fake::standard();
    let ledger = backend.ledger();
    // The first create lands and its reply is lost: the ledger holds an intent with no object
    // id, and the object is there.
    fake.faults_mut().ambiguous_at_call.replace(1);
    let f = failure(apply(&mut fake, &ledger, &chain()));
    assert_eq!(f.at, Some(path("db/run")));
    assert_eq!(fake.object_count(), 1);
    assert_eq!(records(&ledger)[&path("db/run")].state, State::Intent);

    // The next run finds the object by its name, sees our stamp, and takes it: no second
    // create of `db/run`.
    *fake.faults_mut().ambiguous_at_call = None;
    fake.clear_calls();
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(report.resumed, paths(&["db/run"]));
    assert_eq!(report.created, paths(&["api/run", "web/run", "web/public"]));
    let creates_of_db = fake
        .calls()
        .iter()
        .filter(|c| c.op == Op::Create && c.name.starts_with("db-run"))
        .count();
    assert_eq!(creates_of_db, 0);
    assert_eq!(fake.object_count(), 4);
    assert_eq!(records(&ledger)[&path("db/run")].state, State::Confirmed);
    assert!(records(&ledger)[&path("db/run")].physical.is_some());
}

fn a_create_that_loses_a_race_to_our_own_stamp_takes_the_winner(backend: Backend) {
    let mut fake = Fake::standard();
    let ledger = backend.ledger();
    // Before the create lands, another run creates the same object with the same stamp.
    fake.faults_mut().race_next.clone_from(&true);
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(report.resumed, paths(&["db/run"]));
    assert_eq!(report.created, paths(&["api/run", "web/run", "web/public"]));
    assert_eq!(fake.object_count(), 4);
}

fn a_create_over_an_object_that_is_not_ours_stops_the_run(backend: Backend) {
    let mut fake = Fake::standard();
    let ledger = backend.ledger();
    // After the plan and the re-read, an object with the node's name appears with no stamp.
    let at = 2 * chain().nodes().len() + 2; // the re-read is call +1, the create is call +2
    let mut hooked = Hooked::new(&mut fake, at, |fake| {
        fake.add_foreign(&Kind::service(), "db-run", None);
    });
    let f = failure(apply(&mut hooked, &ledger, &chain()));
    drop(hooked);
    assert_eq!(f.at, Some(path("db/run")));
    assert_eq!(f.error, EngineError::NotOurs(path("db/run")));
    assert_eq!(fake.object_count(), 1, "only the foreign object");
}

fn a_foreign_object_found_at_the_re_read_stops_the_run(backend: Backend) {
    let mut fake = Fake::standard();
    let ledger = backend.ledger();
    // After both plans, an object with the node's name appears with no stamp, just before the
    // step reads the node again: the step stops before any intent or create.
    let at = 2 * chain().nodes().len() + 1;
    let mut hooked = Hooked::new(&mut fake, at, |fake| {
        fake.add_foreign(&Kind::service(), "db-run", None);
    });
    let f = failure(apply(&mut hooked, &ledger, &chain()));
    drop(hooked);
    assert_eq!(f.at, Some(path("db/run")));
    assert_eq!(f.error, EngineError::NotOurs(path("db/run")));
    assert!(records(&ledger).is_empty());
    assert_eq!(mutations(&fake), 0, "the run changed nothing");
    assert_eq!(fake.object_count(), 1, "only the foreign object");
}

/// What a later run's plan refuses: the conflicts it names, by path.
fn refused_as(fake: &mut Fake, ledger: &AnyLedger) -> Vec<(String, ConflictKind)> {
    let Outcome::Refused(Refusal::Conflicts(list)) = apply(fake, ledger, &chain()) else {
        panic!("the run should have been refused for conflicts");
    };
    list.into_iter()
        .map(|c| (c.path.to_string(), c.why))
        .collect()
}

fn a_create_refused_over_a_foreign_object_leaves_no_intent(backend: Backend) {
    let mut fake = Fake::standard();
    let ledger = backend.ledger();
    // After the plan and the re-read, an object with the node's name appears with no stamp.
    let at = 2 * chain().nodes().len() + 2;
    let mut hooked = Hooked::new(&mut fake, at, |fake| {
        fake.add_foreign(&Kind::service(), "db-run", None);
    });
    let f = failure(apply(&mut hooked, &ledger, &chain()));
    drop(hooked);
    assert_eq!(f.error, EngineError::NotOurs(path("db/run")));
    // This run created nothing, so it keeps no record of the node.
    assert!(!records(&ledger).contains_key(&path("db/run")));
    // The next run sees a plain foreign object, not one whose stamp was stripped, and changes
    // nothing.
    let before = mutations(&fake);
    assert_eq!(
        refused_as(&mut fake, &ledger),
        [("db/run".to_owned(), ConflictKind::Foreign)]
    );
    assert_eq!(mutations(&fake), before);
    assert_eq!(fake.object_count(), 1, "only the foreign object");
}

fn a_create_refused_over_a_foreign_object_keeps_a_record_another_writer_changed(backend: Backend) {
    let mut fake = Fake::standard();
    let ledger = backend.ledger();
    // Between this run's intent and its read of the object that refused the create, a peer of
    // the same holder (the lease admits no other writer; this is the first run, so fence 1) puts
    // a different record at the path: it is theirs, so it stays.
    let at = 2 * chain().nodes().len() + 2;
    let theirs = &ledger;
    let mut hooked = Hooked::new(&mut fake, at, move |fake| {
        fake.add_foreign(&Kind::service(), "db-run", None);
        let snapshot = theirs.read().expect("read");
        let mine = snapshot.records[&path("db/run")].clone();
        theirs
            .write_fenced(
                Fence::new(1),
                &snapshot.token,
                Change::Put(mine.in_state(State::Reaping)),
            )
            .expect("write");
    });
    let f = failure(apply(&mut hooked, &ledger, &chain()));
    drop(hooked);
    assert_eq!(f.error, EngineError::NotOurs(path("db/run")));
    assert_eq!(records(&ledger)[&path("db/run")].state, State::Reaping);
}

fn a_writer_without_the_lease_cannot_change_a_record_in_the_middle_of_a_run(backend: Backend) {
    let mut fake = Fake::standard();
    let ledger = backend.ledger();
    // Between this run's intent and its read of the object that refused the create, another
    // writer that does not hold the lease tries to put a different record at the path: the
    // ledger refuses it, and the run's own record stays.
    let at = 2 * chain().nodes().len() + 2;
    let theirs = &ledger;
    let attempt: Arc<Mutex<Option<Result<(), LedgerError>>>> = Arc::default();
    let seen = Arc::clone(&attempt);
    let mut hooked = Hooked::new(&mut fake, at, move |fake| {
        fake.add_foreign(&Kind::service(), "db-run", None);
        let snapshot = theirs.read().expect("read");
        let mine = snapshot.records[&path("db/run")].clone();
        let result = theirs.write(&snapshot.token, Change::Put(mine.in_state(State::Reaping)));
        *seen.lock().expect("lock") = Some(result.map(|_| ()));
    });
    let f = failure(apply(&mut hooked, &ledger, &chain()));
    drop(hooked);
    assert_eq!(f.error, EngineError::NotOurs(path("db/run")));
    let refused = attempt.lock().expect("lock").take().expect("the hook ran");
    assert!(
        matches!(refused, Err(LedgerError::Leased { .. })),
        "{refused:?}"
    );
    // Nothing of the refused write is left: the run dropped its own intent, as it does when
    // no other writer is involved.
    assert!(!records(&ledger).contains_key(&path("db/run")));
}

fn a_create_refused_as_existing_while_reads_cannot_see_it_keeps_the_intent(backend: Backend) {
    let mut fake = Fake::standard();
    let ledger = backend.ledger();
    // The racing object exists but reads are lagging: the engine cannot see what refused it.
    {
        let faults = fake.faults_mut();
        *faults.race_next = true;
        *faults.read_lag = 3;
    }
    let f = failure(apply(&mut fake, &ledger, &chain()));
    assert_eq!(
        f.error,
        EngineError::Provider(ProviderError::Refused(CallRefusal::AlreadyExists))
    );
    assert_eq!(records(&ledger)[&path("db/run")].state, State::Intent);
    // Once the lag has passed, the next run takes the object.
    *fake.faults_mut().read_lag = 0;
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(report.resumed, paths(&["db/run"]));
    assert_eq!(fake.object_count(), 4);
}

// ---- the sweep -------------------------------------------------------------------------------

/// What a run does to a world set up by `setup`.
#[derive(Clone)]
enum Act {
    Apply(Lowered),
    Destroy(Lowered),
}

struct Scenario {
    name: &'static str,
    setup: Vec<Lowered>,
    act: Act,
}

fn run_act(
    provider: &mut dyn ucdk_provider::Provider,
    ledger: &dyn ucdk_ledger::Ledger,
    act: &Act,
) -> Outcome {
    match act {
        Act::Apply(desired) => apply(provider, ledger, desired),
        Act::Destroy(desired) => destroy(provider, ledger, desired),
    }
}

fn scenarios() -> Vec<Scenario> {
    let grown = lowered(vec![
        service_of("web", 80, false, &["api"]),
        service_of("api", 81, false, &["db"]),
        service_of("db", 6000, false, &[]),
        service_of("cache", 82, false, &[]),
    ]);
    vec![
        Scenario {
            name: "create everything",
            setup: vec![],
            act: Act::Apply(chain()),
        },
        Scenario {
            name: "update, create and delete in one run",
            setup: vec![chain()],
            act: Act::Apply(grown),
        },
        Scenario {
            name: "create a chain that reads each other's outputs",
            setup: vec![],
            act: Act::Apply(lowered(vec![
                service_reading("web", 80, "API_URL", "api", Output::Url),
                service_reading("api", 81, "DB_HOST", "db", Output::Host),
                service_of("db", 5432, false, &[]),
            ])),
        },
        Scenario {
            name: "destroy everything",
            setup: vec![chain()],
            act: Act::Destroy(chain()),
        },
    ]
}

/// The state a run leaves: the objects and the ledger's records.
fn state_of(
    fake: &Fake,
    ledger: &AnyLedger,
) -> (String, BTreeMap<ucdk_provider::NodePath, Record>) {
    (objects(fake), records(ledger))
}

/// No leaked object (everything the owners hold is recorded) and no duplicate (one object per
/// record, so the object count is the record count).
fn assert_consistent(fake: &mut Fake, ledger: &AnyLedger, context: &str) {
    let recorded = records(ledger);
    let mut ids: Vec<_> = recorded
        .values()
        .filter_map(|r| r.physical.clone())
        .collect();
    ids.sort();
    let owners = ["web", "api", "db", "cache"];
    assert_eq!(
        owned_by_fake(fake, &owners),
        ids,
        "{context}: orphans or missing objects"
    );
    assert_eq!(fake.object_count(), ids.len(), "{context}: duplicates");
    assert!(
        recorded.values().all(|r| r.state == State::Confirmed),
        "{context}: a record is left unresolved"
    );
}

fn crash_sweep_every_call_k_no_duplicates_or_orphans(backend: Backend) {
    for scenario in scenarios() {
        // A clean run: its state is what every crashed-then-rerun world must end in.
        let (mut clean, clean_ledger) = (Fake::standard(), backend.ledger());
        for setup in &scenario.setup {
            done(apply(&mut clean, &clean_ledger, setup));
        }
        let before = mutations(&clean);
        done(run_act(&mut clean, &clean_ledger, &scenario.act));
        let total = mutations(&clean) - before;
        assert!(
            total >= 3,
            "{}: the scenario mutates at least three times",
            scenario.name
        );
        let expected = state_of(&clean, &clean_ledger);

        for ambiguous in [false, true] {
            for k in 1..=total {
                let context = format!("{} (k={k}, ambiguous={ambiguous})", scenario.name);
                let (mut fake, ledger) = (Fake::standard(), backend.ledger());
                for setup in &scenario.setup {
                    done(apply(&mut fake, &ledger, setup));
                }
                let at = (mutations(&fake) + k) as u64;
                {
                    let faults = fake.faults_mut();
                    if ambiguous {
                        *faults.ambiguous_at_call = Some(at);
                    } else {
                        *faults.fail_at_call = Some(at);
                    }
                }
                let crashed = run_act(&mut fake, &ledger, &scenario.act);
                assert!(
                    matches!(crashed, Outcome::Failed(_)),
                    "{context}: {crashed:?}"
                );
                {
                    let faults = fake.faults_mut();
                    *faults.ambiguous_at_call = None;
                    *faults.fail_at_call = None;
                }
                done(run_act(&mut fake, &ledger, &scenario.act));
                assert_eq!(
                    state_of(&fake, &ledger),
                    expected,
                    "{context}: not the clean state"
                );
                assert_consistent(&mut fake, &ledger, &context);
                // And the world is settled: a further run changes nothing.
                let settled = mutations(&fake);
                let again = run_act(&mut fake, &ledger, &scenario.act);
                assert!(matches!(again, Outcome::Done(_)), "{context}: {again:?}");
                assert_eq!(mutations(&fake), settled, "{context}: not settled");
            }
        }
    }
}

/// A holder that died mid-run, whatever stopped it, leaves its lease to lapse. The next run is
/// refused while it holds, changing nothing; once the lease and the skew bound are past, it takes
/// over, and the world ends as a clean run leaves it.
fn a_new_holder_resumes_after_the_dead_one_lapses(
    fake: &mut Fake,
    flaky: &FlakyLedger,
    clock: &ManualClock,
    scenario: &Scenario,
    expected: &(String, BTreeMap<ucdk_provider::NodePath, Record>),
    context: &str,
) {
    let stopped = state_of(fake, &flaky.inner);
    let refused = run_act(fake, flaky, &scenario.act);
    assert!(
        matches!(refused, Outcome::Refused(Refusal::LeaseHeld { .. })),
        "{context}: {refused:?}"
    );
    assert_eq!(
        state_of(fake, &flaky.inner),
        stopped,
        "{context}: a refusal changed something"
    );
    clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
    done(run_act(fake, flaky, &scenario.act));
    assert_eq!(
        &state_of(fake, &flaky.inner),
        expected,
        "{context}: not the clean state"
    );
    assert_consistent(fake, &flaky.inner, context);
}

fn crash_sweep_dead_holder_at_every_ledger_write_a_new_holder_resumes_cleanly(backend: Backend) {
    for scenario in scenarios() {
        let (mut clean, clean_ledger) = (Fake::standard(), backend.ledger());
        for setup in &scenario.setup {
            done(apply(&mut clean, &clean_ledger, setup));
        }
        done(run_act(&mut clean, &clean_ledger, &scenario.act));
        let expected = state_of(&clean, &clean_ledger);

        let (mut counted, counting) = (Fake::standard(), FlakyLedger::new());
        for setup in &scenario.setup {
            done(apply(&mut counted, &counting, setup));
        }
        let before = counting.writes();
        done(run_act(&mut counted, &counting, &scenario.act));
        let total = counting.writes() - before;

        for k in 1..=total {
            let context = format!("{} (holder dies at ledger write {k})", scenario.name);
            let clock = ManualClock::at(1_000_000);
            let (mut fake, flaky) = (
                Fake::standard(),
                FlakyLedger::timed(backend, Arc::new(clock.clone())),
            );
            for setup in &scenario.setup {
                done(apply(&mut fake, &flaky, setup));
            }
            flaky.fail_write_at(flaky.writes() + k);
            flaky.keep_lease(true);
            failure(run_act(&mut fake, &flaky, &scenario.act));
            flaky.fail_write_at(0);
            flaky.keep_lease(false);
            a_new_holder_resumes_after_the_dead_one_lapses(
                &mut fake, &flaky, &clock, &scenario, &expected, &context,
            );
        }
    }
}

fn crash_sweep_dead_holder_at_every_call_k_a_new_holder_resumes_cleanly(backend: Backend) {
    for scenario in scenarios() {
        let (mut clean, clean_ledger) = (Fake::standard(), backend.ledger());
        for setup in &scenario.setup {
            done(apply(&mut clean, &clean_ledger, setup));
        }
        let before = mutations(&clean);
        done(run_act(&mut clean, &clean_ledger, &scenario.act));
        let total = mutations(&clean) - before;
        let expected = state_of(&clean, &clean_ledger);

        for ambiguous in [false, true] {
            for k in 1..=total {
                let context = format!(
                    "{} (holder dies at call k={k}, ambiguous={ambiguous})",
                    scenario.name
                );
                let clock = ManualClock::at(1_000_000);
                let (mut fake, flaky) = (
                    Fake::standard(),
                    FlakyLedger::timed(backend, Arc::new(clock.clone())),
                );
                for setup in &scenario.setup {
                    done(apply(&mut fake, &flaky, setup));
                }
                let at = (mutations(&fake) + k) as u64;
                {
                    let faults = fake.faults_mut();
                    if ambiguous {
                        *faults.ambiguous_at_call = Some(at);
                    } else {
                        *faults.fail_at_call = Some(at);
                    }
                }
                flaky.keep_lease(true);
                failure(run_act(&mut fake, &flaky, &scenario.act));
                flaky.keep_lease(false);
                {
                    let faults = fake.faults_mut();
                    *faults.ambiguous_at_call = None;
                    *faults.fail_at_call = None;
                }
                a_new_holder_resumes_after_the_dead_one_lapses(
                    &mut fake, &flaky, &clock, &scenario, &expected, &context,
                );
            }
        }
    }
}

/// The state a rollback run leaves, without object ids: a rolled-back object comes back under
/// a new id when the next run creates it again.
fn state_without_ids(
    fake: &Fake,
    ledger: &AnyLedger,
) -> (Vec<String>, BTreeMap<ucdk_provider::NodePath, Record>) {
    let mut lines: Vec<String> = objects(fake)
        .lines()
        .map(|l| {
            l.trim()
                .split_once(' ')
                .map_or("", |(_, rest)| rest)
                .to_owned()
        })
        .collect();
    lines.sort();
    let recorded = records(ledger)
        .into_iter()
        .map(|(p, r)| {
            (
                p,
                Record {
                    physical: None,
                    ..r
                },
            )
        })
        .collect();
    (lines, recorded)
}

fn crash_sweep_rollback_every_call_k_no_duplicates_or_orphans(backend: Backend) {
    let (mut clean, clean_ledger) = (Fake::standard(), backend.ledger());
    done(apply(&mut clean, &clean_ledger, &chain()));
    let expected = state_without_ids(&clean, &clean_ledger);

    // The run's own fourth mutation (the last create) fails, either by a refusal or by a lost
    // reply, so the run rolls back. The rollback deletes are calls 5 to 7; a fault of the
    // other kind is injected at each of them.
    for trigger_ambiguous in [false, true] {
        for k in 5..=7u64 {
            let context = format!("rollback (trigger ambiguous={trigger_ambiguous}, k={k})");
            let (mut fake, ledger) = (Fake::standard(), backend.ledger());
            {
                let faults = fake.faults_mut();
                if trigger_ambiguous {
                    *faults.ambiguous_at_call = Some(4);
                    *faults.fail_at_call = Some(k);
                } else {
                    *faults.fail_at_call = Some(4);
                    *faults.ambiguous_at_call = Some(k);
                }
            }
            let crashed = apply_with(&mut fake, &ledger, &chain(), true);
            let f = failure(crashed);
            assert_eq!(f.at, Some(path("web/public")), "{context}");
            assert!(
                !f.report.rollback_failed.is_empty(),
                "{context}: the fault hit a rollback delete"
            );
            {
                let faults = fake.faults_mut();
                *faults.ambiguous_at_call = None;
                *faults.fail_at_call = None;
            }
            done(apply(&mut fake, &ledger, &chain()));
            assert_eq!(
                state_without_ids(&fake, &ledger),
                expected,
                "{context}: not the clean state"
            );
            assert_consistent(&mut fake, &ledger, &context);
            let settled = mutations(&fake);
            done(apply(&mut fake, &ledger, &chain()));
            assert_eq!(mutations(&fake), settled, "{context}: not settled");
        }
    }
}

fn crash_sweep_every_ledger_write_no_duplicates_or_orphans(backend: Backend) {
    for scenario in scenarios() {
        let (mut clean, clean_ledger) = (Fake::standard(), backend.ledger());
        for setup in &scenario.setup {
            done(apply(&mut clean, &clean_ledger, setup));
        }
        done(run_act(&mut clean, &clean_ledger, &scenario.act));
        let expected = state_of(&clean, &clean_ledger);

        // Count the writes of the run.
        let (mut counted, counting) = (Fake::standard(), FlakyLedger::new());
        for setup in &scenario.setup {
            done(apply(&mut counted, &counting, setup));
        }
        let before = counting.writes();
        done(run_act(&mut counted, &counting, &scenario.act));
        let total = counting.writes() - before;
        assert!(
            total >= 6,
            "{}: the run writes the ledger at least six times",
            scenario.name
        );

        for k in 1..=total {
            let context = format!("{} (ledger write {k})", scenario.name);
            let (mut fake, flaky) = (Fake::standard(), FlakyLedger::new());
            for setup in &scenario.setup {
                done(apply(&mut fake, &flaky, setup));
            }
            flaky.fail_write_at(flaky.writes() + k);
            let crashed = run_act(&mut fake, &flaky, &scenario.act);
            let f = failure(crashed);
            assert_eq!(
                f.error,
                EngineError::Ledger(LedgerError::Unavailable("the write was lost".to_owned())),
                "{context}"
            );
            flaky.fail_write_at(0);
            done(run_act(&mut fake, &flaky, &scenario.act));
            assert_eq!(
                state_of(&fake, &flaky.inner),
                expected,
                "{context}: not the clean state"
            );
            assert_consistent(&mut fake, &flaky.inner, &context);
        }
    }
}

// ---- convergence after an out-of-band delete -------------------------------------------------

/// The nodes of the chain, each of which is lost in turn.
const CHAIN_NODES: [&str; 4] = ["db/run", "api/run", "web/run", "web/public"];

/// A world set up by `scenario`, with the object of `lost` (when given) deleted out of band.
fn world_with_loss(backend: Backend, scenario: &Scenario, lost: Option<&str>) -> (Fake, AnyLedger) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    for setup in &scenario.setup {
        done(apply(&mut fake, &ledger, setup));
    }
    if let Some(lost) = lost {
        let record = records(&ledger)[&path(lost)].clone();
        let physical = record.physical.expect("the record names an object");
        fake.delete(&record.kind, &physical)
            .expect("the out-of-band delete");
    }
    (fake, ledger)
}

/// Fault-free runs of the scenario's act, at most three, reach `expected`; then a destroy of
/// what the act manages completes and leaves no object and no record.
fn assert_converges(
    fake: &mut Fake,
    ledger: &AnyLedger,
    act: &Act,
    expected: &(String, BTreeMap<ucdk_provider::NodePath, Record>),
    context: &str,
) {
    let converged = (0..3).any(|_| {
        let outcome = run_act(fake, ledger, act);
        matches!(outcome, Outcome::Done(_)) && state_of(fake, ledger) == *expected
    });
    assert!(converged, "{context}: not the clean state in three runs");
    let (Act::Apply(desired) | Act::Destroy(desired)) = act;
    let outcome = destroy(fake, ledger, desired);
    assert!(
        matches!(outcome, Outcome::Done(_)),
        "{context}: the destroy did not complete: {outcome:?}"
    );
    assert_eq!(fake.object_count(), 0, "{context}: objects left");
    assert!(records(ledger).is_empty(), "{context}: records left");
}

fn crash_sweep_converges_after_out_of_band_delete(backend: Backend) {
    // Every scenario of the sweep, then each of them with one object deleted out of band first.
    let mut cases: Vec<(Scenario, Option<&str>)> = Vec::new();
    for scenario in scenarios() {
        cases.push((scenario, None));
    }
    for lost in CHAIN_NODES {
        cases.push((
            Scenario {
                name: "re-apply after a delete out of band",
                setup: vec![chain()],
                act: Act::Apply(chain()),
            },
            Some(lost),
        ));
    }
    for (scenario, lost) in cases {
        let (mut clean, clean_ledger) = world_with_loss(backend, &scenario, lost);
        let before = mutations(&clean);
        done(run_act(&mut clean, &clean_ledger, &scenario.act));
        let total = mutations(&clean) - before;
        assert!(total >= 1, "{}: the run mutates", scenario.name);
        let expected = state_of(&clean, &clean_ledger);

        for ambiguous in [false, true] {
            for k in 1..=total {
                let context = format!(
                    "{} (lost {lost:?}, k={k}, ambiguous={ambiguous})",
                    scenario.name
                );
                let (mut fake, ledger) = world_with_loss(backend, &scenario, lost);
                let at = (mutations(&fake) + k) as u64;
                {
                    let faults = fake.faults_mut();
                    if ambiguous {
                        *faults.ambiguous_at_call = Some(at);
                    } else {
                        *faults.fail_at_call = Some(at);
                    }
                }
                let crashed = run_act(&mut fake, &ledger, &scenario.act);
                assert!(
                    matches!(crashed, Outcome::Failed(_)),
                    "{context}: {crashed:?}"
                );
                {
                    let faults = fake.faults_mut();
                    *faults.ambiguous_at_call = None;
                    *faults.fail_at_call = None;
                }
                assert_converges(&mut fake, &ledger, &scenario.act, &expected, &context);
            }
        }
    }
}

fn a_recreate_after_an_out_of_band_delete_with_a_lost_reply_converges(backend: Backend) {
    let scenario = Scenario {
        name: "recreate",
        setup: vec![chain()],
        act: Act::Apply(chain()),
    };
    let (mut clean, clean_ledger) = world_with_loss(backend, &scenario, Some("db/run"));
    done(apply(&mut clean, &clean_ledger, &chain()));
    let expected = state_of(&clean, &clean_ledger);

    // The object is deleted out of band; the run that makes it again has its create applied
    // and the reply lost.
    let (mut fake, ledger) = world_with_loss(backend, &scenario, Some("db/run"));
    let old = physical_of(&ledger, "db/run");
    *fake.faults_mut().ambiguous_at_call = Some((mutations(&fake) + 1) as u64);
    let f = failure(apply(&mut fake, &ledger, &chain()));
    assert_eq!(f.at, Some(path("db/run")));
    // The intent it left does not name the deleted object.
    let left = &records(&ledger)[&path("db/run")];
    assert_eq!(left.state, State::Intent);
    assert_ne!(left.physical, Some(old));
    *fake.faults_mut().ambiguous_at_call = None;

    assert_converges(&mut fake, &ledger, &scenario.act, &expected, "recreate");
}

// ---- interleaving ----------------------------------------------------------------------------

/// What a run reports having created, whatever its outcome.
fn created_by(outcome: &Outcome) -> Vec<String> {
    let report: &Report = match outcome {
        Outcome::Done(report) => report,
        Outcome::Failed(f) => &f.report,
        _ => return Vec::new(),
    };
    report.created.iter().map(ToString::to_string).collect()
}

fn a_second_run_that_takes_over_a_stalled_one_at_any_call_creates_nothing_twice(backend: Backend) {
    for scenario in scenarios() {
        let Act::Apply(target) = &scenario.act else {
            continue;
        };
        let (mut clean, clean_ledger) = (Fake::standard(), backend.ledger());
        for setup in &scenario.setup {
            done(apply(&mut clean, &clean_ledger, setup));
        }
        // How many reading and mutating calls the run makes, so every one can be interleaved.
        let mut probe = Hooked::new(&mut clean, usize::MAX, |_| {});
        done(apply(&mut probe, &clean_ledger, target));
        let calls = probe.calls();
        drop(probe);
        let expected = state_of(&clean, &clean_ledger);

        for at in 1..=calls {
            let context = format!("{} (second run before call {at})", scenario.name);
            let clock = ManualClock::at(1_000_000);
            let (mut fake, ledger) = (Fake::standard(), backend.ledger_on(Arc::new(clock.clone())));
            for setup in &scenario.setup {
                done(apply(&mut fake, &ledger, setup));
            }
            let other: Arc<Mutex<Vec<String>>> = Arc::default();
            let first = {
                let (shared, peer, goal) = (Arc::clone(&other), ledger.clone(), target.clone());
                let mut hooked = Hooked::new(&mut fake, at, move |fake| {
                    // The first run stalls past its lease and the skew bound; the second takes
                    // over and runs to the end while the first's call is still to be made.
                    clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
                    let outcome = apply(fake, &peer, &goal);
                    shared.lock().expect("lock").extend(created_by(&outcome));
                });
                apply(&mut hooked, &ledger, target)
            };
            // Whatever the first run made of it, a further run converges.
            let mut created = created_by(&first);
            created.extend(other.lock().expect("lock").iter().cloned());
            done(apply(&mut fake, &ledger, target));
            let mut unique = created.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(
                unique,
                {
                    let mut c = created.clone();
                    c.sort();
                    c
                },
                "{context}: created twice: {created:?}"
            );
            assert_eq!(
                state_of(&fake, &ledger),
                expected,
                "{context}: not the clean state"
            );
            assert_consistent(&mut fake, &ledger, &context);
        }
    }
}

fn a_run_stale_against_the_ledger_writes_again_and_a_busy_ledger_is_reported(backend: Backend) {
    // A write that finds the document changed is tried again after a fresh read.
    let (mut fake, ledger) = (Fake::standard(), common::FlakyLedger::on(backend));
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(report.created.len(), 4);

    // A ledger that stays stale through every attempt stops the run, and the world has not
    // moved on from the intent that could not be written.
    let (mut fake, ledger) = (Fake::standard(), common::FlakyLedger::on(backend));
    ledger.always_stale(true);
    let f = failure(apply(&mut fake, &ledger, &chain()));
    assert_eq!(f.error, EngineError::LedgerBusy);
    assert_eq!(fake.object_count(), 0);
    assert_eq!(ledger.writes(), 5, "five attempts");
}

fn a_ledger_that_cannot_be_read_stops_the_run_before_anything_is_changed(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), common::FlakyLedger::on(backend));
    ledger.fail_reads(true);
    let f = failure(apply(&mut fake, &ledger, &chain()));
    assert_eq!(f.at, None);
    assert!(matches!(
        f.error,
        EngineError::Ledger(LedgerError::Unavailable(_))
    ));
    assert!(fake.calls().is_empty());
}

fn errors_and_refusals_read_as_sentences(backend: Backend) {
    let sentences = [
        EngineError::Provider(ProviderError::Transient("busy".to_owned())).to_string(),
        EngineError::Ledger(LedgerError::Stale).to_string(),
        EngineError::LedgerBusy.to_string(),
        EngineError::NotOurs(path("a/run")).to_string(),
        EngineError::StillPresent(path("a/run")).to_string(),
        EngineError::Unwitnessed(path("a/run")).to_string(),
    ];
    assert_eq!(
        sentences,
        [
            "provider: transient: busy",
            "ledger: the ledger changed since it was read",
            "the ledger kept changing under the run",
            "a/run: the object is not ours (stamp and record)",
            "a/run: the object is there when it should be gone",
            "a/run: no object is read, but the record is an intent that no read has confirmed",
        ]
    );
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    fake.add_foreign(&Kind::service(), "db-run", None);
    let Outcome::Refused(refusal) = apply(&mut fake, &ledger, &chain()) else {
        panic!("expected a refusal");
    };
    assert_eq!(
        refusal.to_string(),
        "refused: db/run: an object with no stamp is there"
    );
}

// ---- every scenario above, once per ledger ---------------------------------------------------

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
    crash_after_intent_adopts_on_rerun,
    a_create_that_loses_a_race_to_our_own_stamp_takes_the_winner,
    a_create_over_an_object_that_is_not_ours_stops_the_run,
    a_foreign_object_found_at_the_re_read_stops_the_run,
    a_create_refused_over_a_foreign_object_leaves_no_intent,
    a_create_refused_over_a_foreign_object_keeps_a_record_another_writer_changed,
    a_writer_without_the_lease_cannot_change_a_record_in_the_middle_of_a_run,
    a_create_refused_as_existing_while_reads_cannot_see_it_keeps_the_intent,
    crash_sweep_every_call_k_no_duplicates_or_orphans,
    crash_sweep_rollback_every_call_k_no_duplicates_or_orphans,
    crash_sweep_every_ledger_write_no_duplicates_or_orphans,
    crash_sweep_dead_holder_at_every_ledger_write_a_new_holder_resumes_cleanly,
    crash_sweep_dead_holder_at_every_call_k_a_new_holder_resumes_cleanly,
    crash_sweep_converges_after_out_of_band_delete,
    a_recreate_after_an_out_of_band_delete_with_a_lost_reply_converges,
    a_second_run_that_takes_over_a_stalled_one_at_any_call_creates_nothing_twice,
    a_run_stale_against_the_ledger_writes_again_and_a_busy_ledger_is_reported,
    a_ledger_that_cannot_be_read_stops_the_run_before_anything_is_changed,
    errors_and_refusals_read_as_sentences,
);
