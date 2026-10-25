// SPDX-License-Identifier: Apache-2.0
//! The lease and its fence, run against every ledger: each test is run on the in-memory ledger
//! and on the file ledger, with a clock the test moves by hand.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{Scratch, path, record};
use ucdk_ledger::{
    Change, FileLedger, LeaseError, Ledger, LedgerError, MAX_CLOCK_SKEW, ManualClock, MemoryLedger,
};

const TTL: Duration = Duration::from_secs(60);

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Runs `check` on a fresh ledger of each kind, with its clock.
fn on_every_ledger(name: &str, check: impl Fn(&dyn Ledger, &ManualClock)) {
    let clock = ManualClock::at(1_000_000);
    check(&MemoryLedger::with_clock(Arc::new(clock.clone())), &clock);
    let scratch = Scratch::new(name);
    let clock = ManualClock::at(1_000_000);
    let file = FileLedger::open_with_clock(scratch.file(), Arc::new(clock.clone())).expect("open");
    check(&file, &clock);
}

fn write_as(
    ledger: &dyn Ledger,
    fence: ucdk_ledger::Fence,
    text: &str,
) -> Result<ucdk_ledger::Token, LedgerError> {
    let token = ledger.read().expect("read").token;
    ledger.write_fenced(fence, &token, Change::Put(record(text)))
}

#[test]
fn a_holder_gets_a_lease_with_its_identity_and_expiry_and_can_write_under_its_fence() {
    on_every_ledger("grant", |ledger, _| {
        let lease = ledger.acquire("alice", TTL).expect("acquire");
        assert_eq!(lease.holder, "alice");
        assert_eq!(lease.expires_at, 1_000_000 + 60_000);
        write_as(ledger, lease.fence, "app/run").expect("the holder writes");
        assert!(
            ledger
                .read()
                .expect("read")
                .records
                .contains_key(&path("app/run"))
        );
    });
}

#[test]
fn a_second_writer_is_refused_while_the_lease_is_held_and_is_told_who_holds_it() {
    on_every_ledger("held", |ledger, clock| {
        ledger.acquire("alice", TTL).expect("acquire");
        clock.advance(secs(10));
        let refused = ledger.acquire("bob", TTL).expect_err("held");
        assert_eq!(
            refused,
            LeaseError::Held {
                holder: "alice".to_owned(),
                remaining: secs(50) + MAX_CLOCK_SKEW
            }
        );
        assert_eq!(
            refused.to_string(),
            "the ledger is leased to alice for at least 55 more second(s)"
        );
    });
}

#[test]
fn a_write_without_a_fence_is_refused_while_a_lease_is_live_and_lands_when_none_is() {
    on_every_ledger("unfenced", |ledger, _| {
        let token = ledger.read().expect("read").token;
        ledger
            .write(&token, Change::Put(record("a/run")))
            .expect("nobody leases: an unfenced write lands");
        let lease = ledger.acquire("alice", TTL).expect("acquire");
        let token = ledger.read().expect("read").token;
        assert_eq!(
            ledger.write(&token, Change::Put(record("b/run"))),
            Err(LedgerError::Leased {
                holder: "alice".to_owned()
            })
        );
        ledger.release(lease.fence).expect("release");
        ledger
            .write(&token, Change::Put(record("b/run")))
            .expect("released: unfenced writes land again");
    });
}

#[test]
fn when_the_lease_expires_a_new_holder_gets_a_higher_fence_and_the_old_holders_write_is_refused() {
    on_every_ledger("expiry", |ledger, clock| {
        let old = ledger.acquire("alice", TTL).expect("acquire");
        write_as(ledger, old.fence, "app/run").expect("alice writes");
        // Alice stalls past her lease and the skew bound; her process is still running.
        clock.advance(TTL + MAX_CLOCK_SKEW);
        let new = ledger.acquire("bob", TTL).expect("the lease lapsed");
        assert!(new.fence > old.fence);
        assert_eq!(
            write_as(ledger, old.fence, "stale/run"),
            Err(LedgerError::Fenced)
        );
        assert_eq!(ledger.renew(old.fence, TTL), Err(LedgerError::Fenced));
        assert_eq!(ledger.release(old.fence), Err(LedgerError::Fenced));
        // Bob is unharmed by Alice's attempts.
        write_as(ledger, new.fence, "bob/run").expect("bob writes");
        let records = ledger.read().expect("read").records;
        assert!(records.contains_key(&path("bob/run")));
        assert!(!records.contains_key(&path("stale/run")));
    });
}

#[test]
fn a_lease_that_lapsed_refuses_its_own_holder_before_anyone_takes_it() {
    on_every_ledger("lapsed", |ledger, clock| {
        let lease = ledger.acquire("alice", TTL).expect("acquire");
        clock.advance(TTL);
        assert_eq!(
            write_as(ledger, lease.fence, "a/run"),
            Err(LedgerError::LeaseLapsed)
        );
        assert_eq!(
            ledger.renew(lease.fence, TTL),
            Err(LedgerError::LeaseLapsed)
        );
        // Nobody took it, so the holder acquires again, and the fence is a new one.
        clock.advance(MAX_CLOCK_SKEW);
        let again = ledger.acquire("alice", TTL).expect("acquire again");
        assert!(again.fence > lease.fence);
        assert_eq!(
            write_as(ledger, lease.fence, "a/run"),
            Err(LedgerError::Fenced)
        );
    });
}

#[test]
fn renewing_keeps_the_lease_past_its_first_expiry() {
    on_every_ledger("renew", |ledger, clock| {
        let lease = ledger.acquire("alice", TTL).expect("acquire");
        clock.advance(secs(50));
        let renewed = ledger.renew(lease.fence, TTL).expect("renew");
        assert_eq!(renewed.fence, lease.fence);
        assert_eq!(renewed.holder, "alice");
        assert_eq!(renewed.expires_at, 1_000_000 + 50_000 + 60_000);
        // Past the first expiry and its skew bound: bob is still refused, alice still writes.
        clock.advance(secs(30) + MAX_CLOCK_SKEW);
        assert!(matches!(
            ledger.acquire("bob", TTL),
            Err(LeaseError::Held { .. })
        ));
        write_as(ledger, lease.fence, "a/run").expect("alice still writes");
    });
}

#[test]
fn releasing_lets_another_holder_in_at_once_with_a_higher_fence() {
    on_every_ledger("release", |ledger, _| {
        let first = ledger.acquire("alice", TTL).expect("acquire");
        ledger.release(first.fence).expect("release");
        let second = ledger.acquire("bob", TTL).expect("no wait after a release");
        assert!(second.fence > first.fence);
        assert_eq!(
            write_as(ledger, first.fence, "a/run"),
            Err(LedgerError::Fenced)
        );
        assert_eq!(ledger.release(first.fence), Err(LedgerError::Fenced));
    });
}

#[test]
fn a_lease_that_was_never_granted_cannot_write_renew_or_release() {
    on_every_ledger("never", |ledger, _| {
        let none = ucdk_ledger::Fence::NONE;
        assert_eq!(write_as(ledger, none, "a/run"), Err(LedgerError::Fenced));
        assert_eq!(ledger.renew(none, TTL), Err(LedgerError::Fenced));
        assert_eq!(ledger.release(none), Err(LedgerError::Fenced));
    });
}

#[test]
fn a_lease_does_not_move_the_documents_token() {
    on_every_ledger("token", |ledger, _| {
        let before = ledger.read().expect("read").token;
        let lease = ledger.acquire("alice", TTL).expect("acquire");
        ledger.renew(lease.fence, TTL).expect("renew");
        assert_eq!(ledger.read().expect("read").token, before);
        ledger.release(lease.fence).expect("release");
        assert_eq!(ledger.read().expect("read").token, before);
    });
}

#[test]
fn a_fenced_write_at_a_stale_token_is_still_stale() {
    on_every_ledger("stale", |ledger, _| {
        let lease = ledger.acquire("alice", TTL).expect("acquire");
        let old = ledger.read().expect("read").token;
        write_as(ledger, lease.fence, "a/run").expect("write");
        assert_eq!(
            ledger.write_fenced(lease.fence, &old, Change::Put(record("b/run"))),
            Err(LedgerError::Stale)
        );
    });
}

#[test]
fn the_lease_is_granted_only_after_the_skew_bound_past_the_lapse() {
    on_every_ledger("skew", |ledger, clock| {
        let lease = ledger.acquire("alice", TTL).expect("acquire");
        // The holder's clock may read up to the bound behind the ledger's: at the lapse by the
        // ledger's clock a taker still waits, and the holder is already refused.
        clock.advance(TTL);
        assert_eq!(
            write_as(ledger, lease.fence, "a/run"),
            Err(LedgerError::LeaseLapsed)
        );
        assert!(matches!(
            ledger.acquire("bob", TTL),
            Err(LeaseError::Held { .. })
        ));
        clock.advance(MAX_CLOCK_SKEW - Duration::from_millis(1));
        assert!(matches!(
            ledger.acquire("bob", TTL),
            Err(LeaseError::Held { .. })
        ));
        clock.advance(Duration::from_millis(1));
        ledger.acquire("bob", TTL).expect("past the bound");
    });
}

#[test]
fn the_lease_and_the_fence_survive_a_reopen_of_the_file_and_the_fence_never_goes_down() {
    let scratch = Scratch::new("reopen");
    let clock = ManualClock::at(1_000_000);
    let first = {
        let ledger =
            FileLedger::open_with_clock(scratch.file(), Arc::new(clock.clone())).expect("open");
        let lease = ledger.acquire("alice", TTL).expect("acquire");
        write_as(&ledger, lease.fence, "a/run").expect("write");
        lease
    };
    let ledger =
        FileLedger::open_with_clock(scratch.file(), Arc::new(clock.clone())).expect("reopen");
    // The process that held it died with no release: the lease is still there for the new one.
    let refused = ledger
        .acquire("bob", TTL)
        .expect_err("alice's lease is on file");
    assert!(matches!(refused, LeaseError::Held { ref holder, .. } if holder == "alice"));
    assert_eq!(write_as(&ledger, first.fence, "b/run").map(|_| ()), Ok(()));
    clock.advance(TTL + MAX_CLOCK_SKEW);
    let second = ledger.acquire("bob", TTL).expect("lapsed");
    assert!(second.fence > first.fence);
    ledger.release(second.fence).expect("release");
    drop(ledger);
    let ledger = FileLedger::open_with_clock(scratch.file(), Arc::new(clock)).expect("reopen");
    let third = ledger.acquire("carol", TTL).expect("released");
    assert!(
        third.fence > second.fence,
        "the fence is kept through a release"
    );
}

#[test]
fn a_write_without_a_fence_lands_once_a_lapsed_lease_is_past_the_skew_bound() {
    on_every_ledger("unfenced_lapsed", |ledger, clock| {
        ledger.acquire("alice", TTL).expect("acquire");
        clock.advance(TTL);
        let token = ledger.read().expect("read").token;
        assert!(matches!(
            ledger.write(&token, Change::Put(record("a/run"))),
            Err(LedgerError::Leased { .. })
        ));
        clock.advance(MAX_CLOCK_SKEW);
        ledger
            .write(&token, Change::Put(record("a/run")))
            .expect("abandoned: an unfenced write lands");
    });
}

#[test]
fn refusals_and_fences_read_as_sentences() {
    assert_eq!(ucdk_ledger::Fence::new(7).to_string(), "f7");
    assert_eq!(
        LeaseError::Unavailable("down".to_owned()).to_string(),
        "the lease is unavailable: down"
    );
    assert!(
        LeaseError::Held {
            holder: "x".to_owned(),
            remaining: Duration::from_millis(1)
        }
        .to_string()
        .ends_with("at least 1 more second(s)")
    );
    assert_eq!(
        LedgerError::Fenced.to_string(),
        "the lease was taken over or released: this writer is fenced out of the ledger"
    );
    assert_eq!(
        LedgerError::LeaseLapsed.to_string(),
        "the lease lapsed before this write"
    );
    assert_eq!(
        LedgerError::Leased {
            holder: "ci-7".to_owned()
        }
        .to_string(),
        "the ledger is leased to ci-7"
    );
}

#[test]
fn the_system_clock_reads_the_wall_time_and_a_manual_clock_only_moves_when_told() {
    use ucdk_ledger::{Clock as _, SystemClock};
    // After the year 2026 began, in milliseconds.
    assert!(SystemClock.now_millis() > 1_767_225_600_000);
    let clock = ManualClock::default();
    assert_eq!(clock.now_millis(), 0);
    clock.advance(secs(2));
    assert_eq!(clock.clone().now_millis(), 2_000);
}
