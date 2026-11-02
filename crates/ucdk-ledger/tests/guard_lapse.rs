// SPDX-License-Identifier: Apache-2.0
//! The edges of the lease guard's clock-dependent decisions, each run on the in-memory and the
//! file ledger with a clock the test moves by hand: when `renew_if_due` renews, what a grant
//! that did not outlive the gap before the guard's first read means, and what a conditional
//! grant whose answer was lost leaves behind.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::hook::AfterGrant;
use common::{AnyLedger, Backend, FlakyLedger, confirmed};
use ucdk_ledger::{
    Change, DEFAULT_LEASE_TTL, Fence, GuardError, LeaseGuard, Ledger, LedgerError, MAX_CLOCK_SKEW,
    ManualClock,
};

const START: u64 = 1_000_000;

fn timed(backend: Backend) -> (AnyLedger, ManualClock) {
    let clock = ManualClock::at(START);
    (backend.ledger_on(Arc::new(clock.clone())), clock)
}

fn expiry(ledger: &dyn Ledger) -> u64 {
    ledger
        .read()
        .expect("read")
        .lease
        .expect("a lease is held")
        .expires_at
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

// ---- renew_if_due at the edges of the ttl ---------------------------------------------------

/// A guard of `ttl_ms` milliseconds, `elapsed` milliseconds later: the answer of `renew_if_due`
/// and the lease's expiry before and after.
fn renew_if_due_after(
    backend: Backend,
    ttl_ms: u64,
    elapsed: u64,
) -> (Result<(), GuardError>, u64, u64) {
    let (ledger, clock) = timed(backend);
    let mut guard = LeaseGuard::acquire(&ledger, "first", ms(ttl_ms))
        .map_err(|e| e.to_string())
        .expect("granted");
    let before = expiry(&ledger);
    assert_eq!(before, START + ttl_ms);
    clock.advance(ms(elapsed));
    let answer = guard.renew_if_due();
    let after = ledger
        .read()
        .expect("read")
        .lease
        .map_or(0, |l| l.expires_at);
    (answer, before, after)
}

fn renew_if_due_rounds_the_half_of_an_odd_ttl_up(backend: Backend) {
    // 3 ms: 2 ms left is more than half (1.5 ms), 1 ms left is less than half.
    let (answer, before, after) = renew_if_due_after(backend, 3, 1);
    assert_eq!(
        (answer, after),
        (Ok(()), before),
        "2 ms of 3 left: not renewed"
    );
    let (answer, _, after) = renew_if_due_after(backend, 3, 2);
    assert_eq!(answer, Ok(()));
    assert_eq!(
        after,
        START + 2 + 3,
        "1 ms of 3 left: renewed to a whole ttl from now"
    );
    // 1 ms: all of it left is not renewed.
    let (answer, before, after) = renew_if_due_after(backend, 1, 0);
    assert_eq!((answer, after), (Ok(()), before));
}

fn renew_if_due_leaves_exactly_half_alone_and_renews_below_it(backend: Backend) {
    let (answer, before, after) = renew_if_due_after(backend, 4, 2);
    assert_eq!(
        (answer, after),
        (Ok(()), before),
        "exactly half left: not renewed"
    );
    let (answer, _, after) = renew_if_due_after(backend, 4, 3);
    assert_eq!(answer, Ok(()));
    assert_eq!(after, START + 3 + 4);
}

fn renew_if_due_on_a_lapsed_lease_with_the_same_fence_is_lost(backend: Backend) {
    // The lapse is the instant `expires_at` is reached, whatever half of the ttl rounds to.
    for (ttl_ms, elapsed) in [(1, 1), (3, 3), (3, 10), (600_000, 600_000)] {
        let (answer, before, after) = renew_if_due_after(backend, ttl_ms, elapsed);
        assert_eq!(
            answer,
            Err(GuardError::Lost),
            "ttl {ttl_ms} ms, {elapsed} ms on"
        );
        assert_eq!(after, before, "a lapsed lease is not extended");
    }
}

fn a_ttl_under_a_millisecond_is_a_lease_that_is_never_live(backend: Backend) {
    // The ledger counts whole milliseconds: such a lease lapses as it is granted, so the grant
    // is already displaced and no guard is made.
    for ttl in [Duration::ZERO, Duration::from_micros(999)] {
        let (ledger, _) = timed(backend);
        let refused = LeaseGuard::acquire(&ledger, "first", ttl).err();
        assert_eq!(refused, Some(GuardError::Lost), "{ttl:?}");
        assert_eq!(expiry(&ledger), START, "the lapsed grant is on the ledger");
    }
}

// ---- a grant that did not outlive the gap before the guard's first read ----------------------

fn a_delayed_read_after_a_takeover_by_another_holder_refuses_the_guard(backend: Backend) {
    let (ledger, clock) = timed(backend);
    let hook = AfterGrant::new(&ledger, || {
        clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
        ledger.acquire("second", DEFAULT_LEASE_TTL).expect("taken");
    });
    let refused = LeaseGuard::acquire(&hook, "first", DEFAULT_LEASE_TTL).err();
    assert_eq!(refused, Some(GuardError::Lost));

    // Nothing of the displaced grant can land, and the new holder is unharmed.
    let before = ledger.read().expect("read");
    assert_eq!(before.fence, Fence::new(2));
    assert_eq!(
        before.lease.as_ref().map(|l| l.holder.as_str()),
        Some("second")
    );
    assert_eq!(
        ledger.write_fenced(
            Fence::new(1),
            &before.token,
            Change::Put(confirmed("app/s1", "x"))
        ),
        Err(LedgerError::Fenced)
    );
    assert_eq!(ledger.read().expect("read"), before);
    let mut second =
        LeaseGuard::acquire(&ledger, "second", DEFAULT_LEASE_TTL).expect("same holder");
    second
        .commit(&Change::Put(confirmed("app/s2", "second-2")))
        .expect("the new holder writes");
}

fn a_delayed_read_after_a_lapse_nobody_took_refuses_the_guard(backend: Backend) {
    let (ledger, clock) = timed(backend);
    let hook = AfterGrant::new(&ledger, || clock.advance(DEFAULT_LEASE_TTL));
    let refused = LeaseGuard::acquire(&hook, "first", DEFAULT_LEASE_TTL).err();
    assert_eq!(refused, Some(GuardError::Lost));
    let before = ledger.read().expect("read");
    assert_eq!(
        ledger.write_fenced(
            Fence::new(1),
            &before.token,
            Change::Put(confirmed("app/s1", "x"))
        ),
        Err(LedgerError::LeaseLapsed),
        "the lapsed grant writes nothing"
    );
    assert_eq!(ledger.read().expect("read"), before);
}

fn a_same_holder_takeover_between_grant_and_read_refuses_the_first_attempt(backend: Backend) {
    let (ledger, _) = timed(backend);
    let hook = AfterGrant::new(&ledger, || {
        ledger
            .acquire("first", DEFAULT_LEASE_TTL)
            .expect("second attempt");
    });
    let refused = LeaseGuard::acquire(&hook, "first", DEFAULT_LEASE_TTL).err();
    assert_eq!(refused, Some(GuardError::Lost));

    let before = ledger.read().expect("read");
    assert_eq!(
        before.fence,
        Fence::new(2),
        "the second attempt's grant stands"
    );
    assert_eq!(
        ledger.write_fenced(
            Fence::new(1),
            &before.token,
            Change::Put(confirmed("app/s1", "x"))
        ),
        Err(LedgerError::Fenced),
        "the refused attempt's fence writes nothing"
    );
    assert_eq!(ledger.read().expect("read"), before);
    ledger
        .write_fenced(
            Fence::new(2),
            &before.token,
            Change::Put(confirmed("app/s2", "second-2")),
        )
        .expect("the second attempt writes");
}

fn an_unfenced_write_between_grant_and_read_is_refused_by_the_live_lease(backend: Backend) {
    let (ledger, _) = timed(backend);
    let hook = AfterGrant::new(&ledger, || {
        let token = ledger.read().expect("read").token;
        let refused = ledger.write(&token, Change::Put(confirmed("app/other", "other-1")));
        assert!(
            matches!(refused, Err(LedgerError::Leased { .. })),
            "{refused:?}"
        );
    });
    let mut guard = LeaseGuard::acquire(&hook, "first", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("granted: nothing got in");
    let read = ledger.read().expect("read");
    assert_eq!(
        guard.token(),
        &read.token,
        "the baseline is the document as read"
    );
    assert!(read.records.is_empty());
    guard
        .commit(&Change::Put(confirmed("app/s1", "first-1")))
        .expect("writes under its fence");
}

fn an_unfenced_write_after_the_lapse_refuses_the_guard_and_moves_the_document(backend: Backend) {
    let (ledger, clock) = timed(backend);
    let hook = AfterGrant::new(&ledger, || {
        clock.advance(DEFAULT_LEASE_TTL + MAX_CLOCK_SKEW);
        let token = ledger.read().expect("read").token;
        ledger
            .write(&token, Change::Put(confirmed("app/other", "other-1")))
            .expect("past the skew bound an unfenced write is admitted");
    });
    let refused = LeaseGuard::acquire(&hook, "first", DEFAULT_LEASE_TTL).err();
    assert_eq!(refused, Some(GuardError::Lost));
    let read = ledger.read().expect("read");
    assert_eq!(read.records.len(), 1, "the unfenced write stands");
    assert_eq!(
        ledger.write_fenced(
            Fence::new(1),
            &read.token,
            Change::Put(confirmed("app/s1", "x"))
        ),
        Err(LedgerError::LeaseLapsed)
    );
}

fn a_write_under_the_granted_fence_between_grant_and_read_is_the_baseline(backend: Backend) {
    // A second process of the same holder that learned the fence and wrote before the first
    // read: the guard's fence is still the granted one, and its baseline the document as read.
    let (ledger, _) = timed(backend);
    let hook = AfterGrant::new(&ledger, || {
        let read = ledger.read().expect("read");
        ledger
            .write_fenced(
                read.fence,
                &read.token,
                Change::Put(confirmed("app/s0", "early")),
            )
            .expect("fenced write");
    });
    let mut guard = LeaseGuard::acquire(&hook, "first", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("granted");
    let read = ledger.read().expect("read");
    assert_eq!(guard.fence(), read.fence);
    assert_eq!(
        guard.token(),
        &read.token,
        "the baseline includes the early write"
    );
    assert_eq!(read.records.len(), 1);
    guard
        .commit(&Change::Put(confirmed("app/s1", "first-1")))
        .expect("writes under its fence");
    assert_eq!(ledger.read().expect("read").records.len(), 2);
}

// ---- a conditional grant that landed and lost its answer -------------------------------------

fn a_conditional_grant_that_landed_and_lost_its_answer_is_recovered_by_acquiring(backend: Backend) {
    let clock = ManualClock::at(START);
    let flaky = FlakyLedger::on_clock(backend, Arc::new(clock.clone()));
    let mut guard = LeaseGuard::acquire(&flaky, "first", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("granted");
    guard
        .commit(&Change::Put(confirmed("app/s1", "first-1")))
        .expect("a write");
    let token = guard.token().clone();
    let old = guard.fence();
    clock.advance(DEFAULT_LEASE_TTL + Duration::from_secs(1));

    flaky.lose_acquire_if_answer(true);
    assert!(matches!(
        guard.reacquire_if_unchanged(&token, old),
        Err(GuardError::Unconfirmed { .. })
    ));
    assert_eq!(flaky.acquire_if_calls(), 1, "the grant reached the ledger");

    // The new fence is on the ledger, held by this holder.
    let landed = flaky.read().expect("read");
    assert_eq!(landed.fence, Fence::new(old.value() + 1));
    assert_eq!(
        landed.lease.as_ref().map(|l| l.holder.as_str()),
        Some("first")
    );

    // The guard did not learn the new fence, so it confirms no work under the old one.
    assert_eq!(
        guard.commit(&Change::Put(confirmed("app/s2", "first-2"))),
        Err(GuardError::Lost)
    );
    assert_eq!(flaky.read().expect("read"), landed, "nothing was written");

    // The next ordinary acquire by the same holder takes the lease under a newer fence.
    flaky.lose_acquire_if_answer(false);
    let mut again = LeaseGuard::acquire(&flaky, "first", DEFAULT_LEASE_TTL)
        .map_err(|e| e.to_string())
        .expect("recovered");
    assert_eq!(again.fence(), Fence::new(old.value() + 2));
    again
        .commit(&Change::Put(confirmed("app/s2", "first-2")))
        .expect("the recovered guard writes");
    assert_eq!(flaky.read().expect("read").records.len(), 2);
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
    renew_if_due_rounds_the_half_of_an_odd_ttl_up,
    renew_if_due_leaves_exactly_half_alone_and_renews_below_it,
    renew_if_due_on_a_lapsed_lease_with_the_same_fence_is_lost,
    a_ttl_under_a_millisecond_is_a_lease_that_is_never_live,
    a_delayed_read_after_a_takeover_by_another_holder_refuses_the_guard,
    a_delayed_read_after_a_lapse_nobody_took_refuses_the_guard,
    a_same_holder_takeover_between_grant_and_read_refuses_the_first_attempt,
    an_unfenced_write_between_grant_and_read_is_refused_by_the_live_lease,
    an_unfenced_write_after_the_lapse_refuses_the_guard_and_moves_the_document,
    a_write_under_the_granted_fence_between_grant_and_read_is_the_baseline,
    a_conditional_grant_that_landed_and_lost_its_answer_is_recovered_by_acquiring,
);
