// SPDX-License-Identifier: Apache-2.0
//! Crash safety and races: a run that stops at any call, with the call failing or its reply
//! lost, is followed by a run that leaves the world exactly as a clean run would; and runs
//! that interleave create nothing twice.

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::{
    FlakyLedger, Hooked, apply, apply_with, chain, destroy, done, lowered, mutations, objects,
    owned_by_fake, path, paths, records, service_of,
};
use ucdk_engine::{EngineError, Outcome, Refusal, Report};
use ucdk_ledger::{Change, Ledger as _, LedgerError, MemoryLedger, Record, State};
use ucdk_plan::{ConflictKind, Lowered};
use ucdk_provider::{Kind, ProviderError, Refusal as CallRefusal};
use ucdk_provider_fake::{Fake, Op};

fn failure(outcome: Outcome) -> ucdk_engine::Failure {
    match outcome {
        Outcome::Failed(f) => f,
        other => panic!("the run should have failed: {other:?}"),
    }
}

// ---- one crashed create ----------------------------------------------------------------------

#[test]
fn crash_after_intent_adopts_on_rerun() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
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

#[test]
fn a_create_that_loses_a_race_to_our_own_stamp_takes_the_winner() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
    // Before the create lands, another run creates the same object with the same stamp.
    fake.faults_mut().race_next.clone_from(&true);
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(report.resumed, paths(&["db/run"]));
    assert_eq!(report.created, paths(&["api/run", "web/run", "web/public"]));
    assert_eq!(fake.object_count(), 4);
}

#[test]
fn a_create_over_an_object_that_is_not_ours_stops_the_run() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
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

#[test]
fn a_foreign_object_found_at_the_re_read_stops_the_run() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
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
fn refused_as(fake: &mut Fake, ledger: &MemoryLedger) -> Vec<(String, ConflictKind)> {
    let Outcome::Refused(Refusal::Conflicts(list)) = apply(fake, ledger, &chain()) else {
        panic!("the run should have been refused for conflicts");
    };
    list.into_iter()
        .map(|c| (c.path.to_string(), c.why))
        .collect()
}

#[test]
fn a_create_refused_over_a_foreign_object_leaves_no_intent() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
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

#[test]
fn a_create_refused_over_a_foreign_object_keeps_a_record_another_writer_changed() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
    // Between this run's intent and its read of the object that refused the create, another
    // writer puts a different record at the path: it is theirs, so it stays.
    let at = 2 * chain().nodes().len() + 2;
    let theirs = &ledger;
    let mut hooked = Hooked::new(&mut fake, at, move |fake| {
        fake.add_foreign(&Kind::service(), "db-run", None);
        let snapshot = theirs.read().expect("read");
        let mine = snapshot.records[&path("db/run")].clone();
        theirs
            .write(&snapshot.token, Change::Put(mine.in_state(State::Reaping)))
            .expect("write");
    });
    let f = failure(apply(&mut hooked, &ledger, &chain()));
    drop(hooked);
    assert_eq!(f.error, EngineError::NotOurs(path("db/run")));
    assert_eq!(records(&ledger)[&path("db/run")].state, State::Reaping);
}

#[test]
fn a_create_refused_as_existing_while_reads_cannot_see_it_keeps_the_intent() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
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
            name: "destroy everything",
            setup: vec![chain()],
            act: Act::Destroy(chain()),
        },
    ]
}

/// The state a run leaves: the objects and the ledger's records.
fn state_of(
    fake: &Fake,
    ledger: &MemoryLedger,
) -> (String, BTreeMap<ucdk_provider::NodePath, Record>) {
    (objects(fake), records(ledger))
}

/// No leaked object (everything the owners hold is recorded) and no duplicate (one object per
/// record, so the object count is the record count).
fn assert_consistent(fake: &mut Fake, ledger: &MemoryLedger, context: &str) {
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

#[test]
fn crash_sweep_every_call_k_no_duplicates_or_orphans() {
    for scenario in scenarios() {
        // A clean run: its state is what every crashed-then-rerun world must end in.
        let (mut clean, clean_ledger) = (Fake::standard(), MemoryLedger::new());
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
                let (mut fake, ledger) = (Fake::standard(), MemoryLedger::new());
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

/// The state a rollback run leaves, without object ids: a rolled-back object comes back under
/// a new id when the next run creates it again.
fn state_without_ids(
    fake: &Fake,
    ledger: &MemoryLedger,
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

#[test]
fn crash_sweep_rollback_every_call_k_no_duplicates_or_orphans() {
    let (mut clean, clean_ledger) = (Fake::standard(), MemoryLedger::new());
    done(apply(&mut clean, &clean_ledger, &chain()));
    let expected = state_without_ids(&clean, &clean_ledger);

    // The run's own fourth mutation (the last create) fails, either by a refusal or by a lost
    // reply, so the run rolls back. The rollback deletes are calls 5 to 7; a fault of the
    // other kind is injected at each of them.
    for trigger_ambiguous in [false, true] {
        for k in 5..=7u64 {
            let context = format!("rollback (trigger ambiguous={trigger_ambiguous}, k={k})");
            let (mut fake, ledger) = (Fake::standard(), MemoryLedger::new());
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

#[test]
fn crash_sweep_every_ledger_write_no_duplicates_or_orphans() {
    for scenario in scenarios() {
        let (mut clean, clean_ledger) = (Fake::standard(), MemoryLedger::new());
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

#[test]
fn interleaved_applies_create_nothing_twice() {
    for scenario in scenarios() {
        let Act::Apply(target) = &scenario.act else {
            continue;
        };
        let (mut clean, clean_ledger) = (Fake::standard(), MemoryLedger::new());
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
            let (mut fake, ledger) = (Fake::standard(), MemoryLedger::new());
            for setup in &scenario.setup {
                done(apply(&mut fake, &ledger, setup));
            }
            let other: Arc<Mutex<Vec<String>>> = Arc::default();
            let first = {
                let (shared, peer, goal) = (Arc::clone(&other), ledger.clone(), target.clone());
                let mut hooked = Hooked::new(&mut fake, at, move |fake| {
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

#[test]
fn a_run_stale_against_the_ledger_writes_again_and_a_busy_ledger_is_reported() {
    // A write that finds the document changed is tried again after a fresh read.
    let (mut fake, ledger) = (Fake::standard(), common::FlakyLedger::new());
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(report.created.len(), 4);

    // A ledger that stays stale through every attempt stops the run, and the world has not
    // moved on from the intent that could not be written.
    let (mut fake, ledger) = (Fake::standard(), common::FlakyLedger::new());
    ledger.always_stale(true);
    let f = failure(apply(&mut fake, &ledger, &chain()));
    assert_eq!(f.error, EngineError::LedgerBusy);
    assert_eq!(fake.object_count(), 0);
    assert_eq!(ledger.writes(), 5, "five attempts");
}

#[test]
fn a_ledger_that_cannot_be_read_stops_the_run_before_anything_is_changed() {
    let (mut fake, ledger) = (Fake::standard(), common::FlakyLedger::new());
    ledger.fail_reads(true);
    let f = failure(apply(&mut fake, &ledger, &chain()));
    assert_eq!(f.at, None);
    assert!(matches!(
        f.error,
        EngineError::Ledger(LedgerError::Unavailable(_))
    ));
    assert!(fake.calls().is_empty());
}

#[test]
fn errors_and_refusals_read_as_sentences() {
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
    let (mut fake, ledger) = (Fake::standard(), MemoryLedger::new());
    fake.add_foreign(&Kind::service(), "db-run", None);
    let Outcome::Refused(refusal) = apply(&mut fake, &ledger, &chain()) else {
        panic!("expected a refusal");
    };
    assert_eq!(
        refusal.to_string(),
        "refused: db/run: an object with no stamp is there"
    );
}
