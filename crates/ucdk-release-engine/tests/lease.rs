// SPDX-License-Identifier: Apache-2.0
//! The run's lease while a Test action runs: kept alive on the progress the build system
//! reports, taken again after a silence longer than the lease when nothing else happened to the
//! ledger, and never used to record a result after another holder had it. Time is a
//! `ManualClock` that the ledger and the fake build system share; no test waits.

mod common;

use std::time::Duration;

use common::{Rig, TTL, Watched};
use ucdk_ledger::{Ledger, MAX_CLOCK_SKEW, State};
use ucdk_release_fake::FakeBuildSystem;
use ucdk_release_proto::v1::{BuildSystemKind, Reason, Verdict};

const HOLDER: &str = "run:r1/build";

#[test]
fn a_test_longer_than_the_ttl_that_reports_progress_keeps_its_lease() {
    let mut rig = Rig::new("progress");
    let start = common::START;
    // Eight beats of 100 s, a test of 800 s under a lease of 600 s.
    rig.build.beats(8, Duration::from_secs(100));
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    assert_eq!(
        rig.clock_ms(),
        start + 800_000,
        "more time than the lease passed"
    );
    let snapshot = rig.ledger.inner.read().expect("read");
    assert_eq!(
        snapshot.fence.value(),
        1,
        "the lease was never lost and taken again"
    );
    assert_eq!(rig.ledger.acquire_ifs(), 0);
    // Renewed before the invocation; on the beats that find less than half left (the fourth, at
    // 400 s, and the eighth, at 800 s, the lease having been renewed at 400 s); and once before
    // the result was recorded.
    assert_eq!(rig.ledger.renews(), 4);
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Confirmed);
    assert_eq!(rig.ledger.lease(), None, "and given back");
}

#[test]
fn a_beat_renews_only_when_less_than_half_the_ttl_remains() {
    // Two beats of 100 s: 400 s of 600 s are left at the last, which is more than half, so the
    // only renewals are the two the engine makes itself.
    let mut rig = Rig::new("beats_quiet");
    rig.build.beats(2, Duration::from_secs(100));
    assert_eq!(
        rig.handle("build", &["action:test"]).verdict,
        Verdict::Succeeded
    );
    assert_eq!(rig.ledger.renews(), 2);

    // A beat when exactly half is left does not renew either; one millisecond past does.
    let mut rig = Rig::new("beats_edge");
    rig.build.beats(1, Duration::from_secs(300));
    assert_eq!(
        rig.handle("build", &["action:test"]).verdict,
        Verdict::Succeeded
    );
    assert_eq!(rig.ledger.renews(), 2);
    let mut rig = Rig::new("beats_edge_past");
    rig.build.beats(1, Duration::from_millis(300_001));
    assert_eq!(
        rig.handle("build", &["action:test"]).verdict,
        Verdict::Succeeded
    );
    assert_eq!(rig.ledger.renews(), 3);
}

#[test]
fn a_silent_test_longer_than_the_ttl_is_confirmed_by_reacquiring() {
    let mut rig = Rig::new("silent");
    rig.build.silent_for(Duration::from_secs(700));
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    // The lease lapsed during the silence; nothing else touched the ledger, so it was taken
    // again, under a new fence, to record the result.
    assert_eq!(rig.ledger.acquire_ifs(), 1);
    assert_eq!(rig.ledger.inner.read().expect("read").fence.value(), 2);
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Confirmed);
    assert_eq!(rig.ledger.lease(), None);
}

/// Runs `action:test` of `build` with `hook` done just before the build system is asked.
fn run_with_hook<'a>(
    rig: &mut Rig,
    hook: impl FnMut() + Send + 'a,
) -> ucdk_release_engine::Outcome {
    let mut build = FakeBuildSystem::new(BuildSystemKind::Buck2, &rig.clock);
    let mut watched = Watched::new(&mut build, hook);
    rig.handle_with(&mut watched, "build", &["action:test"])
}

#[test]
fn a_test_whose_lease_was_taken_over_is_not_confirmed() {
    let mut rig = Rig::new("taken_over");
    let (ledger, clock) = (rig.ledger.clone(), rig.clock.clone());
    let outcome = run_with_hook(&mut rig, || {
        // The run stalls past its lease and the skew bound; another holder takes the lease.
        clock.advance(TTL + MAX_CLOCK_SKEW);
        ledger.inner.acquire("someone-else", TTL).expect("taken");
    });
    assert_eq!(outcome.verdict, Verdict::Interrupted);
    assert_eq!(outcome.reason, Reason::LeaseLost);
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Intent, "the result was not recorded");
    let (holder, _) = rig.ledger.lease().expect("the other holder's lease");
    assert_eq!(
        holder, "someone-else",
        "and its lease was not given back by us"
    );
}

#[test]
fn a_beat_after_a_takeover_stops_the_invocation_and_records_nothing() {
    let mut rig = Rig::new("beat_takeover");
    let (ledger, clock) = (rig.ledger.clone(), rig.clock.clone());
    let mut build = FakeBuildSystem::new(BuildSystemKind::Buck2, &rig.clock);
    build.beats(3, Duration::from_secs(1));
    let mut watched = Watched::new(&mut build, || {
        clock.advance(TTL + MAX_CLOCK_SKEW);
        ledger.inner.acquire("someone-else", TTL).expect("taken");
    });
    let outcome = rig.handle_with(&mut watched, "build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Interrupted);
    assert_eq!(outcome.reason, Reason::LeaseLost);
    assert!(
        outcome.message.contains("nothing more was recorded"),
        "{}",
        outcome.message
    );
    assert!(
        outcome.actions.is_empty(),
        "the action has no report: it did not finish"
    );
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Intent);
    assert_eq!(
        rig.ledger.acquire_ifs(),
        0,
        "a stopped invocation is not recorded by reacquiring"
    );
}

#[test]
fn a_second_attempt_of_the_same_holder_fences_a_silent_first_attempt() {
    let mut rig = Rig::new("second_attempt");
    let (ledger, clock) = (rig.ledger.clone(), rig.clock.clone());
    let second = std::sync::Mutex::new(None);
    let outcome = run_with_hook(&mut rig, || {
        // The first attempt is silent for longer than its lease. Attempt 2, the same holder,
        // takes the lease at once (it is the holder the lease names), without touching the
        // document.
        clock.advance(TTL + Duration::from_secs(1));
        *second.lock().expect("lock") = Some(ledger.inner.acquire(HOLDER, TTL).expect("attempt 2"));
    });
    assert_eq!(outcome.verdict, Verdict::Interrupted);
    assert_eq!(outcome.reason, Reason::LeaseLost);
    let (_, state) = rig.ledger.action("build", "test").expect("the record");
    assert_eq!(state, State::Intent, "attempt 1 recorded nothing");

    // Attempt 2 is not fenced out: its lease is the current one, and it writes under it.
    let second = second
        .into_inner()
        .expect("lock")
        .expect("attempt 2 took the lease");
    let snapshot = rig.ledger.inner.read().expect("read");
    assert_eq!(snapshot.fence, second.fence);
    assert_eq!(snapshot.lease.expect("held").holder, HOLDER);
    rig.ledger
        .inner
        .renew(second.fence, TTL)
        .expect("attempt 2 still holds the lease");
}

#[test]
fn a_lease_another_holder_has_refuses_the_request_and_changes_nothing() {
    let mut rig = Rig::new("held");
    rig.ledger.inner.acquire("someone-else", TTL).expect("held");
    let before = rig.ledger.inner.read().expect("read");
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Refused);
    assert_eq!(outcome.reason, Reason::LeaseHeld);
    assert!(
        outcome.message.contains("someone-else"),
        "{}",
        outcome.message
    );
    assert_eq!(rig.ledger.inner.read().expect("read"), before);
    assert!(rig.build.commands().is_empty());
}

#[test]
fn a_grant_whose_answer_was_lost_is_unconfirmed_and_nothing_is_run() {
    let mut rig = Rig::new("unconfirmed");
    rig.ledger.lose_acquire_answer(true);
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Indeterminate);
    assert_eq!(outcome.reason, Reason::LeaseUnconfirmed);
    assert!(rig.build.commands().is_empty());
    assert!(rig.ledger.records().is_empty());
    // The lease may be ours; the same holder takes it again at once.
    rig.ledger.lose_acquire_answer(false);
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
}

#[test]
fn a_release_that_cannot_be_confirmed_does_not_change_what_the_request_did() {
    let mut rig = Rig::new("release_lost");
    rig.ledger.fail_release(true);
    let outcome = rig.handle("build", &["action:test"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
    assert_eq!(rig.ledger.lease().expect("still held").0, HOLDER);
    // A rerun of the same stage of the same run takes the lease at once.
    rig.ledger.fail_release(false);
    let again = rig.handle("build", &["action:test"]);
    assert_eq!(again.verdict, Verdict::Noop);
    assert_eq!(rig.ledger.lease(), None);
}
