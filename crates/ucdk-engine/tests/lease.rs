// SPDX-License-Identifier: Apache-2.0
//! The ledger lease in the engine: an apply or a destroy takes it for its run and writes under
//! its fence, so two runs on one ledger cannot both write, and a run that lost the lease cannot
//! write again. Each test runs on the in-memory and the file ledger.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{
    AnyLedger, Backend, FlakyLedger, Hooked, apply, cell, chain, destroy, done, mutations, path,
    plan_of, planned, records,
};
use ucdk_engine::{DEFAULT_LEASE_TTL, Engine, EngineError, Mode, Outcome, Refusal, Request};
use ucdk_ledger::{
    Change, Clock, Fence, LeaseError, Ledger, LedgerError, MAX_CLOCK_SKEW, ManualClock, State,
};
use ucdk_plan::Lowered;
use ucdk_provider::Provider;
use ucdk_provider_fake::Fake;

/// Plans, then applies the plan under its own hash, as the holder `name`.
fn apply_as(
    name: &str,
    provider: &mut dyn Provider,
    ledger: &dyn Ledger,
    desired: &Lowered,
) -> Outcome {
    let plan = planned(plan_of(provider, ledger, desired));
    let plan_hash = plan.hash().clone();
    Engine::new(provider, ledger, cell())
        .with_holder(name)
        .handle(Request::Apply {
            desired: desired.clone(),
            plan,
            plan_hash,
            rollback: false,
        })
}

fn timed(backend: Backend) -> (AnyLedger, ManualClock) {
    let clock = ManualClock::at(1_000_000);
    (backend.ledger_on(Arc::new(clock.clone())), clock)
}

fn failure(outcome: Outcome) -> ucdk_engine::Failure {
    match outcome {
        Outcome::Failed(f) => f,
        other => panic!("the run should have failed: {other:?}"),
    }
}

fn second_writer_is_refused_while_another_run_is_in_progress(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    // In the middle of the first run (at its first re-read, after the plan it was approved
    // under), a second run on the same ledger and the same cloud is started.
    let at = 2 * chain().nodes().len() + 1;
    let second_ledger = ledger.clone();
    let second: Arc<Mutex<Option<Outcome>>> = Arc::default();
    let seen = Arc::clone(&second);
    let mut hooked = Hooked::new(&mut fake, at, move |fake| {
        *seen.lock().expect("lock") = Some(apply_as("second", fake, &second_ledger, &chain()));
    });
    let first = apply_as("first", &mut hooked, &ledger, &chain());
    drop(hooked);
    done(first);
    let refused = second.lock().expect("lock").take().expect("the hook ran");
    let Outcome::Refused(Refusal::LeaseHeld { holder, remaining }) = refused else {
        panic!("the second run should be refused for the lease: {refused:?}");
    };
    assert_eq!(holder, "first");
    assert!(remaining > Duration::ZERO && remaining <= DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
    assert_eq!(fake.object_count(), 4, "the second run created nothing");
}

fn a_refused_run_changes_nothing_and_a_plan_or_status_needs_no_lease(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    ledger
        .acquire("someone", DEFAULT_LEASE_TTL)
        .expect("acquire");
    let refused = apply(&mut fake, &ledger, &chain());
    assert!(matches!(
        refused,
        Outcome::Refused(Refusal::LeaseHeld { .. })
    ));
    assert_eq!(mutations(&fake), 0);
    assert!(records(&ledger).is_empty());
    // Reading is not writing: a plan and a status are answered while another run holds it.
    let plan = plan_of(&mut fake, &ledger, &chain());
    assert!(matches!(plan, Outcome::Planned(_)), "{plan:?}");
    let status =
        Engine::new(&mut fake, &ledger, cell()).handle(Request::Status { desired: chain() });
    assert!(matches!(status, Outcome::Status(_)), "{status:?}");
    // A destroy is a writer too.
    assert!(matches!(
        destroy(&mut fake, &ledger, &chain()),
        Outcome::Refused(Refusal::LeaseHeld { .. })
    ));
}

fn a_run_gives_the_lease_back_when_it_ends_even_when_it_failed(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    fake.faults_mut().fail_at_call.replace(4);
    failure(apply(&mut fake, &ledger, &chain()));
    // No waiting: the failed run released it.
    ledger
        .acquire("next", DEFAULT_LEASE_TTL)
        .and_then(|lease| {
            ledger
                .release(lease.fence)
                .map_err(|e| LeaseError::Unavailable(e.to_string()))
        })
        .expect("the lease is free after a failed run");
    *fake.faults_mut().fail_at_call = None;
    done(apply(&mut fake, &ledger, &chain()));
    ledger
        .acquire("after", DEFAULT_LEASE_TTL)
        .expect("free after a finished run");
}

fn a_run_that_cannot_reach_the_lease_fails_with_no_effect(backend: Backend) {
    let (mut fake, flaky) = (Fake::standard(), FlakyLedger::on(backend));
    flaky.fail_acquire(true);
    let f = failure(apply(&mut fake, &flaky, &chain()));
    assert_eq!(f.at, None);
    assert!(matches!(
        f.error,
        EngineError::Ledger(LedgerError::Unavailable(_))
    ));
    assert_eq!(mutations(&fake), 0);
}

fn a_run_whose_lease_was_taken_over_stops_at_its_next_write_and_the_new_holder_is_unharmed(
    backend: Backend,
) {
    let (mut fake, (ledger, clock)) = (Fake::standard(), timed(backend));
    // The first run stalls at its first re-read for longer than its lease and the skew bound;
    // its process is still running. A second run starts in the gap and finishes the work.
    let at = 2 * chain().nodes().len() + 1;
    let second_ledger = ledger.clone();
    let hook_clock = clock.clone();
    let taken: Arc<Mutex<Option<Outcome>>> = Arc::default();
    let seen = Arc::clone(&taken);
    let mut hooked = Hooked::new(&mut fake, at, move |fake| {
        hook_clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
        *seen.lock().expect("lock") = Some(apply_as("second", fake, &second_ledger, &chain()));
    });
    let stalled = apply_as("first", &mut hooked, &ledger, &chain());
    drop(hooked);
    assert!(matches!(
        taken.lock().expect("lock").take().expect("the hook ran"),
        Outcome::Done(_)
    ));
    let after_second = (records(&ledger), common::objects(&fake), mutations(&fake));

    // The old holder is refused by the ledger at its next write and stops there.
    let f = failure(stalled);
    assert_eq!(f.error, EngineError::LeaseLost);
    assert_eq!(f.at, Some(path("db/run")));
    assert!(f.report.created.is_empty());
    // It changed neither the ledger nor the cloud after the new holder finished.
    assert_eq!(
        (records(&ledger), common::objects(&fake), mutations(&fake)),
        after_second
    );
    assert!(
        records(&ledger)
            .values()
            .all(|r| r.state == State::Confirmed)
    );
    assert_eq!(fake.object_count(), 4);
}

fn a_lost_lease_is_not_rolled_back_over_the_new_holders_work(backend: Backend) {
    let (mut fake, (ledger, clock)) = (Fake::standard(), timed(backend));
    let at = 2 * chain().nodes().len() + 1;
    let second_ledger = ledger.clone();
    let mut hooked = Hooked::new(&mut fake, at, move |fake| {
        clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
        done(apply_as("second", fake, &second_ledger, &chain()));
    });
    let plan = planned(plan_of(&mut hooked, &ledger, &chain()));
    let plan_hash = plan.hash().clone();
    let outcome = Engine::new(&mut hooked, &ledger, cell()).handle(Request::Apply {
        desired: chain(),
        plan,
        plan_hash,
        rollback: true,
    });
    drop(hooked);
    let f = failure(outcome);
    assert_eq!(f.error, EngineError::LeaseLost);
    assert!(f.report.rolled_back.is_empty() && f.report.rollback_failed.is_empty());
    assert_eq!(
        fake.object_count(),
        4,
        "the new holder's objects are all still there"
    );
}

/// A clock that moves by `step` every time it is read, so a run's ledger work takes time.
#[derive(Debug)]
struct Ticking {
    now: AtomicU64,
    step: u64,
}

impl Clock for Ticking {
    fn now_millis(&self) -> u64 {
        self.now.fetch_add(self.step, Ordering::SeqCst)
    }
}

fn a_run_that_lasts_longer_than_its_lease_keeps_it_by_renewing(backend: Backend) {
    // Each clock read takes 8 s and the lease lasts 50 s: no step comes near the lease, and
    // the whole run takes more than twice as long as it.
    let clock = Arc::new(Ticking {
        now: AtomicU64::new(1_000_000),
        step: 8_000,
    });
    let ledger = backend.ledger_on(clock.clone());
    let mut fake = Fake::standard();
    let plan = planned(plan_of(&mut fake, &ledger, &chain()));
    let plan_hash = plan.hash().clone();
    let started = clock.now.load(Ordering::SeqCst);
    let outcome = Engine::new(&mut fake, &ledger, cell())
        .with_lease_ttl(Duration::from_secs(50))
        .handle(Request::Apply {
            desired: chain(),
            plan,
            plan_hash,
            rollback: false,
        });
    done(outcome);
    let took = clock.now.load(Ordering::SeqCst) - started;
    assert!(
        took > 2 * 50_000,
        "the run took {took} ms, not several leases"
    );
    assert_eq!(fake.object_count(), 4);
}

fn a_destroy_is_fenced_like_an_apply(backend: Backend) {
    let (mut fake, (ledger, clock)) = (Fake::standard(), timed(backend));
    done(apply(&mut fake, &ledger, &chain()));
    let at = 2 * chain().nodes().len() + 1;
    let second_ledger = ledger.clone();
    let mut hooked = Hooked::new(&mut fake, at, move |fake| {
        clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
        done(destroy(fake, &second_ledger, &chain()));
    });
    let stalled = destroy(&mut hooked, &ledger, &chain());
    drop(hooked);
    assert_eq!(failure(stalled).error, EngineError::LeaseLost);
    assert!(records(&ledger).is_empty());
    assert_eq!(fake.object_count(), 0);
}

fn the_default_holder_names_the_process_and_a_holder_can_be_named(backend: Backend) {
    let (mut fake, ledger) = (Fake::standard(), backend.ledger());
    let held = ledger
        .acquire("someone-else", DEFAULT_LEASE_TTL)
        .expect("acquire");
    let Outcome::Refused(Refusal::LeaseHeld { holder, .. }) = apply(&mut fake, &ledger, &chain())
    else {
        panic!("refused");
    };
    assert_eq!(holder, "someone-else");
    // The run that is refused names itself to the next: the default is the process.
    ledger.release(held.fence).expect("release");
    let at = 2 * chain().nodes().len() + 1;
    let probe = ledger.clone();
    let name: Arc<Mutex<String>> = Arc::default();
    let seen = Arc::clone(&name);
    let mut hooked = Hooked::new(&mut fake, at, move |_| {
        if let Err(LeaseError::Held { holder, .. }) = probe.acquire("probe", Duration::from_secs(1))
        {
            *seen.lock().expect("lock") = holder;
        }
    });
    done(apply(&mut hooked, &ledger, &chain()));
    assert_eq!(
        *name.lock().expect("lock"),
        format!("ucdk-{}", std::process::id())
    );
}

fn a_fence_the_ledger_never_granted_cannot_write(backend: Backend) {
    let ledger = backend.ledger();
    let token = ledger.read().expect("read").token;
    let result = ledger.write_fenced(Fence::NONE, &token, Change::Remove(path("a/run")));
    assert_eq!(result, Err(LedgerError::Fenced));
}

#[test]
fn errors_and_refusals_of_the_lease_read_as_sentences() {
    assert_eq!(
        Refusal::LeaseHeld {
            holder: "ci-7".to_owned(),
            remaining: Duration::from_secs(95)
        }
        .to_string(),
        "refused: the ledger is leased to ci-7; try again when that run has ended \
         (its lease can be taken within 95 second(s) at most)"
    );
    assert!(
        Refusal::LeaseHeld {
            holder: "ci-7".to_owned(),
            remaining: Duration::from_millis(5)
        }
        .to_string()
        .contains("within 1 second(s)")
    );
    assert!(
        EngineError::LeaseLost
            .to_string()
            .contains("lease was lost")
    );
    assert_eq!(
        EngineError::from(LedgerError::Fenced),
        EngineError::LeaseLost
    );
    assert_eq!(
        EngineError::from(LedgerError::LeaseLapsed),
        EngineError::LeaseLost
    );
    assert_eq!(
        EngineError::from(LedgerError::Stale),
        EngineError::Ledger(LedgerError::Stale)
    );
    let _ = Mode::Apply;
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
    second_writer_is_refused_while_another_run_is_in_progress,
    a_refused_run_changes_nothing_and_a_plan_or_status_needs_no_lease,
    a_run_gives_the_lease_back_when_it_ends_even_when_it_failed,
    a_run_that_cannot_reach_the_lease_fails_with_no_effect,
    a_run_whose_lease_was_taken_over_stops_at_its_next_write_and_the_new_holder_is_unharmed,
    a_lost_lease_is_not_rolled_back_over_the_new_holders_work,
    a_run_that_lasts_longer_than_its_lease_keeps_it_by_renewing,
    a_destroy_is_fenced_like_an_apply,
    the_default_holder_names_the_process_and_a_holder_can_be_named,
    a_fence_the_ledger_never_granted_cannot_write,
);
