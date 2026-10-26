// SPDX-License-Identifier: Apache-2.0
//! The lease guard: one holder takes the lease of a ledger, writes under its fence, renews it,
//! and gives it back, so two holders on one ledger cannot both write and a holder that lost the
//! lease cannot write again. Each test runs on the in-memory and the file ledger.
//!
//! The work a lease protects is a [`Counter`]: a call with an effect outside the ledger, whose
//! count says how far a holder got. A "run" below is what a consumer of the guard does: acquire,
//! then for each step renew, make the call and commit the record, and release however it ended.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{AnyLedger, Backend, Counter, FlakyLedger, Hooked, confirmed, path};
use ucdk_ledger::{
    Change, Clock, DEFAULT_LEASE_TTL, Fence, GuardError, LeaseGuard, Ledger, LedgerError,
    MAX_CLOCK_SKEW, ManualClock, State, default_holder,
};

/// Why a run stopped before its last step.
#[derive(Debug, PartialEq)]
enum Stop {
    /// A call failed.
    Effect,
    Guard(GuardError),
}

impl From<GuardError> for Stop {
    fn from(e: GuardError) -> Self {
        Self::Guard(e)
    }
}

/// What a run did: how its steps ended, and what giving the lease back answered.
struct Run {
    work: Result<(), Stop>,
    released: Result<(), GuardError>,
}

/// The steps of a run: before each, the lease is renewed; then a call; then the record of the
/// step is committed, with a physical id that names who wrote it. The `fail_at`th call fails.
fn steps(
    guard: &mut LeaseGuard<'_>,
    call: &dyn Fn() -> u64,
    count: u64,
    fail_at: Option<u64>,
) -> Result<(), Stop> {
    for i in 1..=count {
        guard.renew()?;
        if Some(call()) == fail_at {
            return Err(Stop::Effect);
        }
        let id = format!("{}-{i}", guard.holder());
        guard.commit(&Change::Put(confirmed(&format!("app/s{i}"), &id)))?;
    }
    Ok(())
}

/// A whole run as `holder`: acquire, the steps, and a release however the steps ended.
fn run(
    ledger: &dyn Ledger,
    holder: &str,
    call: &dyn Fn() -> u64,
    count: u64,
    fail_at: Option<u64>,
) -> Result<Run, GuardError> {
    let mut guard = LeaseGuard::acquire(ledger, holder, DEFAULT_LEASE_TTL)?;
    let work = steps(&mut guard, call, count, fail_at);
    let released = guard.release();
    Ok(Run { work, released })
}

fn finished(run: Result<Run, GuardError>) {
    let run = run
        .map_err(|e| e.to_string())
        .expect("the lease is granted");
    assert_eq!(run.work, Ok(()));
    assert_eq!(run.released, Ok(()));
}

fn timed(backend: Backend) -> (AnyLedger, ManualClock) {
    let clock = ManualClock::at(1_000_000);
    (backend.ledger_on(Arc::new(clock.clone())), clock)
}

fn records_of(ledger: &dyn Ledger) -> Vec<(String, String)> {
    ledger
        .read()
        .expect("read")
        .records
        .values()
        .map(|r| {
            (
                r.path.to_string(),
                r.physical
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
            )
        })
        .collect()
}

fn second_writer_is_refused_while_another_run_is_in_progress(backend: Backend) {
    let ledger = backend.ledger();
    // In the middle of the first run, at its first call, a second holder tries the lease.
    let refused: Arc<Mutex<Option<GuardError>>> = Arc::default();
    let hooked = {
        let (ledger, refused) = (ledger.clone(), Arc::clone(&refused));
        Hooked::new(1, move || {
            *refused.lock().expect("lock") =
                LeaseGuard::acquire(&ledger, "second", DEFAULT_LEASE_TTL).err();
        })
    };
    finished(run(&ledger, "first", &|| hooked.call(), 3, None));
    let Some(GuardError::Held { holder, remaining }) = refused.lock().expect("lock").take() else {
        panic!("the second holder should be refused for the lease");
    };
    assert_eq!(holder, "first");
    assert!(remaining > Duration::ZERO && remaining <= DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
    assert_eq!(hooked.calls(), 3, "the first run went on to the end");
    assert_eq!(records_of(&ledger).len(), 3);
}

fn a_refused_acquire_changes_nothing_and_a_read_needs_no_lease(backend: Backend) {
    let ledger = backend.ledger();
    ledger
        .acquire("someone", DEFAULT_LEASE_TTL)
        .expect("acquire");
    let before = ledger.read().expect("read");
    let work = Counter::default();
    let refused = run(&ledger, "run", &|| work.call(), 2, None);
    assert!(
        matches!(refused, Err(GuardError::Held { ref holder, .. }) if holder == "someone"),
        "{:?}",
        refused.err()
    );
    assert_eq!(work.calls(), 0);
    // Token, fence, lease and records are as they were; the clock does not move.
    assert_eq!(ledger.read().expect("read"), before);
    // Reading is not writing: a read is answered while another holder holds the lease.
    assert_eq!(before.lease.expect("held").holder, "someone");
}

fn a_run_gives_the_lease_back_when_it_ends_even_when_it_failed(backend: Backend) {
    let ledger = backend.ledger();
    let work = Counter::default();
    let failed = run(&ledger, "first", &|| work.call(), 4, Some(2)).expect("granted");
    assert_eq!(failed.work, Err(Stop::Effect));
    assert_eq!(failed.released, Ok(()));
    // No waiting: the failed run released it.
    let mut next = LeaseGuard::acquire(&ledger, "next", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("the lease is free after a failed run");
    assert_eq!(next.release(), Ok(()));
    finished(run(&ledger, "first", &|| work.call(), 4, None));
    assert!(ledger.acquire("after", DEFAULT_LEASE_TTL).is_ok());
}

fn a_run_that_cannot_confirm_the_lease_is_not_safe_to_run_again(backend: Backend) {
    let flaky = FlakyLedger::on(backend);
    flaky.fail_acquire(true);
    let work = Counter::default();
    let refused = run(&flaky, "first", &|| work.call(), 2, None);
    let Err(GuardError::Unconfirmed { why }) = refused else {
        panic!("{:?}", refused.err());
    };
    assert!(why.contains("the lease is down"), "{why}");
    assert_eq!(work.calls(), 0);
    assert_eq!(flaky.writes(), 0);
}

fn a_grant_whose_answer_was_lost_is_taken_again_at_once_by_the_same_holder_only(backend: Backend) {
    let flaky = FlakyLedger::on(backend);
    flaky.lose_acquire_answer(true);
    let work = Counter::default();
    let lost = run(&flaky, "first", &|| work.call(), 3, None);
    assert!(matches!(lost, Err(GuardError::Unconfirmed { .. })));
    flaky.lose_acquire_answer(false);
    // The lease is on the ledger, in the first holder's name.
    let other = run(&flaky, "second", &|| work.call(), 3, None);
    assert!(
        matches!(&other, Err(GuardError::Held { holder, .. }) if holder == "first"),
        "{:?}",
        other.err()
    );
    assert_eq!(work.calls(), 0);
    finished(run(&flaky, "first", &|| work.call(), 3, None));
    assert_eq!(work.calls(), 3);
}

fn a_failed_release_after_a_failed_run_is_not_safe_to_run_again(backend: Backend) {
    let flaky = FlakyLedger::on(backend);
    flaky.fail_release(true);
    let work = Counter::default();
    let failed = run(&flaky, "first", &|| work.call(), 3, Some(1)).expect("granted");
    assert_eq!(
        failed.work,
        Err(Stop::Effect),
        "the call's own failure is kept"
    );
    let Err(GuardError::Unconfirmed { why }) = &failed.released else {
        panic!("{:?}", failed.released);
    };
    assert!(why.contains("the release was lost"), "{why}");
    let text = failed.released.expect_err("unconfirmed").to_string();
    assert!(text.contains("may still be held"), "{text}");

    // The lease is still on the ledger: another holder is refused, the same one is not.
    flaky.fail_release(false);
    assert!(matches!(
        run(&flaky, "second", &|| work.call(), 3, None),
        Err(GuardError::Held { .. })
    ));
    finished(run(&flaky, "first", &|| work.call(), 3, None));
}

fn a_failed_release_after_a_finished_run_leaves_the_run_done(backend: Backend) {
    let flaky = FlakyLedger::on(backend);
    flaky.fail_release(true);
    let work = Counter::default();
    let run = run(&flaky, "first", &|| work.call(), 3, None).expect("granted");
    assert_eq!(run.work, Ok(()), "every step was committed");
    assert!(matches!(run.released, Err(GuardError::Unconfirmed { .. })));
    assert_eq!(records_of(&flaky).len(), 3);
}

fn a_release_that_could_not_be_confirmed_can_be_asked_again(backend: Backend) {
    let flaky = FlakyLedger::on(backend);
    let mut guard = LeaseGuard::acquire(&flaky, "first", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("granted");
    flaky.fail_release(true);
    assert!(matches!(
        guard.release(),
        Err(GuardError::Unconfirmed { .. })
    ));
    flaky.fail_release(false);
    assert_eq!(
        guard.release(),
        Ok(()),
        "the guard stayed held, so the call is repeated"
    );
    assert_eq!(guard.release(), Err(GuardError::NotHeld));
    assert_eq!(guard.renew(), Err(GuardError::NotHeld));
    let write = guard.commit(&Change::Remove(path("app/s1")));
    assert_eq!(write, Err(GuardError::NotHeld));
    assert_eq!(flaky.writes(), 0, "a released guard writes nothing");
}

fn a_run_whose_lease_was_taken_over_stops_at_its_next_write_and_the_new_holder_is_unharmed(
    backend: Backend,
) {
    let (ledger, clock) = timed(backend);
    // The first run stalls at its first call for longer than its lease and the skew bound; its
    // process is still running. A second run starts in the gap and finishes the work.
    let hooked = {
        let (ledger, clock) = (ledger.clone(), clock.clone());
        Hooked::new(1, move || {
            clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
            let second = Counter::default();
            finished(run(&ledger, "second", &|| second.call(), 3, None));
        })
    };
    let stalled = run(&ledger, "first", &|| hooked.call(), 3, None).expect("granted");
    let after_second = ledger.read().expect("read");

    // The old holder is refused by the ledger at its next write and stops there.
    assert_eq!(stalled.work, Err(Stop::Guard(GuardError::Lost)));
    assert_eq!(
        stalled.released,
        Err(GuardError::Lost),
        "nothing of the new holder's is given back"
    );
    assert_eq!(hooked.calls(), 1, "no further call was made");
    // It changed nothing after the new holder finished: every record is the second's.
    assert_eq!(ledger.read().expect("read"), after_second);
    assert!(after_second.records.values().all(|r| {
        r.state == State::Confirmed
            && r.physical
                .as_ref()
                .is_some_and(|p| p.as_str().starts_with("second-"))
    }));
    assert_eq!(after_second.records.len(), 3);
    assert_eq!(
        after_second.lease, None,
        "the second run gave the lease back"
    );
}

fn a_holder_that_lost_its_lease_writes_nothing_more(backend: Backend) {
    // Lost by a takeover (the fence is no longer the current one) and by a lapse that nobody
    // took (the fence is current but the time is past): both end the same way.
    for takeover in [true, false] {
        let clock = ManualClock::at(1_000_000);
        let flaky = FlakyLedger::on_clock(backend, Arc::new(clock.clone()));
        let mut guard = LeaseGuard::acquire(&flaky, "first", DEFAULT_LEASE_TTL)
            .map_err(|e| e.to_string())
            .expect("granted");
        guard
            .commit(&Change::Put(confirmed("app/s1", "first-1")))
            .expect("a write under the live lease");
        clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
        if takeover {
            let mut second = LeaseGuard::acquire(&flaky, "second", DEFAULT_LEASE_TTL)
                .map_err(|e| e.to_string())
                .expect("granted after the lapse");
            second
                .commit(&Change::Put(confirmed("app/s2", "second-2")))
                .expect("the new holder writes");
        }
        let before = (records_of(&flaky), flaky.read().expect("read").token);
        let writes_before = flaky.writes();

        let refused = guard.commit(&Change::Put(confirmed("app/s3", "first-3")));
        assert_eq!(refused, Err(GuardError::Lost), "takeover: {takeover}");
        let attempts = flaky.writes() - writes_before;
        // Every later call answers Lost, and none reaches the ledger.
        assert_eq!(
            guard.commit(&Change::Remove(path("app/s1"))),
            Err(GuardError::Lost)
        );
        assert_eq!(guard.renew(), Err(GuardError::Lost));
        assert_eq!(guard.release(), Err(GuardError::Lost));
        assert_eq!(
            flaky.writes() - writes_before,
            attempts,
            "the guard made no further attempt"
        );
        assert_eq!(
            (records_of(&flaky), flaky.read().expect("read").token),
            before,
            "the document is as it was"
        );
    }
}

fn renew_after_a_takeover_is_lost(backend: Backend) {
    let (ledger, clock) = timed(backend);
    let mut first = LeaseGuard::acquire(&ledger, "first", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("granted");
    clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
    let second = ledger
        .acquire("second", DEFAULT_LEASE_TTL)
        .expect("granted");
    assert_eq!(first.renew(), Err(GuardError::Lost));
    // The new holder's lease is untouched by the refused renewal.
    assert_eq!(
        ledger.read().expect("read").lease.expect("held").fence,
        second.fence
    );
}

fn release_after_a_takeover_is_lost(backend: Backend) {
    let (ledger, clock) = timed(backend);
    let mut first = LeaseGuard::acquire(&ledger, "first", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("granted");
    clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
    let second = ledger
        .acquire("second", DEFAULT_LEASE_TTL)
        .expect("granted");
    assert_eq!(first.release(), Err(GuardError::Lost));
    // The new holder's lease was not given back by the old holder's release.
    let held = ledger.read().expect("read").lease.expect("still held");
    assert_eq!((held.holder.as_str(), held.fence), ("second", second.fence));
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
    // Each clock read takes 8 s and the lease lasts 50 s: no step comes near the lease, and the
    // whole run takes more than twice as long as it.
    let clock = Arc::new(Ticking {
        now: AtomicU64::new(1_000_000),
        step: 8_000,
    });
    let ledger = backend.ledger_on(clock.clone());
    let started = clock.now.load(Ordering::SeqCst);
    let work = Counter::default();
    let mut guard = LeaseGuard::acquire(&ledger, "first", Duration::from_secs(50))
        .map_err(|e| e.to_string())
        .expect("granted");
    steps(&mut guard, &|| work.call(), 6, None).expect("every step is under a live lease");
    assert_eq!(guard.release(), Ok(()));
    let took = clock.now.load(Ordering::SeqCst) - started;
    assert!(
        took > 2 * 50_000,
        "the run took {took} ms, not several leases"
    );
    assert_eq!(work.calls(), 6);
}

fn a_fence_the_ledger_never_granted_cannot_write(backend: Backend) {
    let ledger = backend.ledger();
    let token = ledger.read().expect("read").token;
    let result = ledger.write_fenced(Fence::NONE, &token, Change::Remove(path("a/run")));
    assert_eq!(result, Err(LedgerError::Fenced));
}

fn commit_retries_a_stale_token_up_to_five_times_then_is_busy(backend: Backend) {
    let flaky = FlakyLedger::on(backend);
    let mut guard = LeaseGuard::acquire(&flaky, "first", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("granted");
    let change = Change::Put(confirmed("app/s1", "first-1"));
    // Four stale answers and the fifth attempt lands.
    flaky.stale_for(4);
    assert_eq!(guard.commit(&change), Ok(()));
    assert_eq!(flaky.writes(), 5);
    // Five stale answers are every attempt there is: busy, and nothing is written.
    let before = records_of(&flaky);
    flaky.stale_for(5);
    let writes = flaky.writes();
    assert_eq!(
        guard.commit(&Change::Put(confirmed("app/s2", "first-2"))),
        Err(GuardError::Busy)
    );
    assert_eq!(flaky.writes() - writes, 5);
    assert_eq!(records_of(&flaky), before);
    // Busy is not a lost lease: the next commit goes through.
    assert_eq!(
        guard.commit(&Change::Put(confirmed("app/s2", "first-2"))),
        Ok(())
    );
}

fn a_commit_that_cannot_read_the_ledger_says_so(backend: Backend) {
    let flaky = FlakyLedger::on(backend);
    let mut guard = LeaseGuard::acquire(&flaky, "first", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("granted");
    flaky.fail_reads(true);
    let result = guard.commit(&Change::Put(confirmed("app/s1", "first-1")));
    assert!(
        matches!(result, Err(GuardError::Ledger(LedgerError::Unavailable(ref why))) if why.contains("reads are down")),
        "{result:?}"
    );
    flaky.fail_reads(false);
    assert_eq!(
        guard.commit(&Change::Put(confirmed("app/s1", "first-1"))),
        Ok(())
    );
    assert_eq!(flaky.writes(), 1);
}

fn the_default_holder_names_the_process_and_a_holder_can_be_named(backend: Backend) {
    let ledger = backend.ledger();
    // A holder is told who has the lease by the name it was taken under.
    let named = LeaseGuard::acquire(&ledger, "someone-else", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("granted");
    assert_eq!(named.holder(), "someone-else");
    let Err(GuardError::Held { holder, .. }) =
        LeaseGuard::acquire(&ledger, "probe", Duration::from_secs(1))
    else {
        panic!("refused");
    };
    assert_eq!(holder, "someone-else");
    // The default name is the process and a random number, new on every call.
    let (one, two) = (default_holder(), default_holder());
    let prefix = format!("ucdk-{}-", std::process::id());
    assert!(
        one.starts_with(&prefix) && two.starts_with(&prefix),
        "{one} {two}"
    );
    assert_ne!(one, two, "the name is not the process alone");
    let fresh = backend.ledger();
    let defaulted = LeaseGuard::acquire(&fresh, one.clone(), DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("granted");
    assert_eq!(defaulted.holder(), one);
    assert_eq!(defaulted.fence().value(), 1);
}

#[test]
fn errors_and_refusals_of_the_lease_read_as_sentences() {
    assert_eq!(
        GuardError::Held {
            holder: "ci-7".to_owned(),
            remaining: Duration::from_secs(95)
        }
        .to_string(),
        "the ledger is leased to ci-7; try again when that holder is done \
         (its lease can be taken within 95 second(s) at most)"
    );
    assert!(
        GuardError::Held {
            holder: "ci-7".to_owned(),
            remaining: Duration::from_millis(5)
        }
        .to_string()
        .contains("within 1 second(s)")
    );
    assert_eq!(
        GuardError::Lost.to_string(),
        "the lease was lost (it lapsed, or another holder took it): nothing more is written"
    );
    assert_eq!(
        GuardError::Unconfirmed {
            why: "the answer was lost".to_owned()
        }
        .to_string(),
        "the lease may still be held (the answer was lost): act again as the same holder, \
         or wait for the lease to lapse"
    );
    assert_eq!(
        GuardError::Ledger(LedgerError::Stale).to_string(),
        "ledger: the ledger changed since it was read"
    );
    assert_eq!(
        GuardError::Busy.to_string(),
        "the ledger kept changing under the writer"
    );
    assert_eq!(GuardError::NotHeld.to_string(), "the lease is not held");
}

#[test]
fn a_ledger_opened_to_read_grants_no_lease() {
    let scratch = common::Scratch::new("guard_read_only");
    let writer = ucdk_ledger::FileLedger::open(scratch.file()).expect("open");
    common::put(&writer, Change::Put(confirmed("app/s1", "id-1")));
    drop(writer);
    let reader = ucdk_ledger::FileLedger::open_read_only(scratch.file()).expect("open read-only");
    let refused = LeaseGuard::acquire(&reader, "first", DEFAULT_LEASE_TTL).err();
    assert_eq!(refused, Some(GuardError::Ledger(LedgerError::ReadOnly)));
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
    a_refused_acquire_changes_nothing_and_a_read_needs_no_lease,
    a_run_gives_the_lease_back_when_it_ends_even_when_it_failed,
    a_run_that_cannot_confirm_the_lease_is_not_safe_to_run_again,
    a_grant_whose_answer_was_lost_is_taken_again_at_once_by_the_same_holder_only,
    a_failed_release_after_a_failed_run_is_not_safe_to_run_again,
    a_failed_release_after_a_finished_run_leaves_the_run_done,
    a_release_that_could_not_be_confirmed_can_be_asked_again,
    a_run_whose_lease_was_taken_over_stops_at_its_next_write_and_the_new_holder_is_unharmed,
    a_holder_that_lost_its_lease_writes_nothing_more,
    renew_after_a_takeover_is_lost,
    release_after_a_takeover_is_lost,
    a_run_that_lasts_longer_than_its_lease_keeps_it_by_renewing,
    a_fence_the_ledger_never_granted_cannot_write,
    commit_retries_a_stale_token_up_to_five_times_then_is_busy,
    a_commit_that_cannot_read_the_ledger_says_so,
    the_default_holder_names_the_process_and_a_holder_can_be_named,
);
