// SPDX-License-Identifier: Apache-2.0
//! The compare-and-swap contract, run against every ledger: each test below is run once on the
//! in-memory ledger and once on the file ledger, so the two cannot drift apart.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::release::{
    golden_records, grant, granted, publication, put_release, read_release, refused, run, seal,
};
use common::{Scratch, confirmed, path, put, record};
use ucdk_ledger::{
    Change, Fence, FileLedger, Holding, LeaseError, LeaseState, Ledger, LedgerError,
    MAX_CLOCK_SKEW, ManualClock, MemoryLedger, Origin, State,
};
use ucdk_release::codec::{Phase, keys_of};
use ucdk_release::transitions::Kind;

/// Runs `check` on a fresh ledger of each kind.
fn on_every_ledger(name: &str, check: impl Fn(&dyn Ledger)) {
    check(&MemoryLedger::new());
    let scratch = Scratch::new(name);
    check(&FileLedger::open(scratch.file()).expect("open"));
}

/// The instant every clocked ledger starts at, in milliseconds.
const START: u64 = 1_000_000;
const TTL: Duration = Duration::from_secs(60);

/// Runs `check` on a fresh ledger of each kind, each on a clock the test moves by hand.
fn on_every_clocked_ledger(name: &str, check: impl Fn(&dyn Ledger, &ManualClock)) {
    let clock = ManualClock::at(START);
    check(&MemoryLedger::with_clock(Arc::new(clock.clone())), &clock);
    let scratch = Scratch::new(name);
    let clock = ManualClock::at(START);
    let file = FileLedger::open_with_clock(scratch.file(), Arc::new(clock.clone())).expect("open");
    check(&file, &clock);
}

/// Moves `clock` to `to` milliseconds.
fn move_to(clock: &ManualClock, to: u64) {
    use ucdk_ledger::Clock as _;
    clock.advance(Duration::from_millis(to - clock.now_millis()));
}

#[test]
fn a_new_ledger_is_empty_at_version_zero() {
    on_every_ledger("empty", |ledger| {
        let snapshot = ledger.read().expect("read");
        assert!(snapshot.records.is_empty());
        assert_eq!(snapshot.token.to_string(), "v0");
    });
}

#[test]
fn a_write_at_the_current_token_lands_and_returns_the_next_token() {
    on_every_ledger("write", |ledger| {
        let first = ledger.read().expect("read").token;
        let second = ledger
            .write(&first, Change::Put(record("app/run")))
            .expect("write");
        assert_ne!(first, second);
        let snapshot = ledger.read().expect("read");
        assert_eq!(snapshot.token, second);
        assert_eq!(snapshot.records.len(), 1);
    });
}

#[test]
fn a_write_at_a_stale_token_is_refused_and_changes_nothing() {
    on_every_ledger("stale", |ledger| {
        let old = ledger.read().expect("read").token;
        put(ledger, Change::Put(record("app/run")));
        let result = ledger.write(&old, Change::Put(record("db/run")));
        assert_eq!(result, Err(LedgerError::Stale));
        let records = ledger.read().expect("read").records;
        assert_eq!(records.len(), 1);
        assert!(records.contains_key(&path("app/run")));
    });
}

#[test]
fn put_replaces_a_record_and_remove_drops_it_and_removing_nothing_is_not_an_error() {
    on_every_ledger("put_remove", |ledger| {
        put(ledger, Change::Put(record("app/run")));
        put(ledger, Change::Put(confirmed("app/run", "id-1")));
        let records = ledger.read().expect("read").records;
        assert_eq!(records.len(), 1);
        assert_eq!(records[&path("app/run")].state, State::Confirmed);
        put(ledger, Change::Remove(path("app/run")));
        assert!(ledger.read().expect("read").records.is_empty());
        put(ledger, Change::Remove(path("app/run")));
    });
}

#[test]
fn a_read_reports_the_fence_the_lease_and_the_clock() {
    on_every_clocked_ledger("read_lease", |ledger, clock| {
        let fresh = ledger.read().expect("read");
        assert_eq!(fresh.fence, Fence::NONE);
        assert_eq!(fresh.lease, None);
        assert_eq!(fresh.at, START);

        let lease = ledger.acquire("alice", TTL).expect("acquire");
        clock.advance(Duration::from_secs(10));
        let held = ledger.read().expect("read");
        assert_eq!(held.fence, lease.fence);
        assert_eq!(held.fence.value(), 1);
        assert_eq!(
            held.lease,
            Some(Holding {
                holder: "alice".to_owned(),
                fence: lease.fence,
                expires_at: START + 60_000,
            })
        );
        assert_eq!(held.at, START + 10_000, "the reading follows the clock");
        assert_eq!(held.token, fresh.token, "a lease does not move the token");

        // A renewal moves the expiry, and the next grant moves the fence.
        ledger.renew(lease.fence, TTL).expect("renew");
        let renewed = ledger.read().expect("read");
        assert_eq!(renewed.lease.expect("held").expires_at, START + 70_000);
        ledger.release(lease.fence).expect("release");
        assert_eq!(
            ledger.acquire("bob", TTL).expect("acquire").fence.value(),
            2
        );
        assert_eq!(ledger.read().expect("read").fence.value(), 2);
    });
}

#[test]
fn a_read_after_a_release_reports_no_lease() {
    on_every_clocked_ledger("read_released", |ledger, _| {
        let lease = ledger.acquire("alice", TTL).expect("acquire");
        ledger.release(lease.fence).expect("release");
        let after = ledger.read().expect("read");
        assert_eq!(after.lease, None);
        assert_eq!(after.fence, lease.fence, "the fence never goes down");
    });
}

#[test]
fn holding_state_agrees_with_what_acquire_does() {
    on_every_clocked_ledger("holding_state", |ledger, clock| {
        ledger.acquire("alice", TTL).expect("acquire");
        let expires = START + 60_000;
        let skew = u64::try_from(MAX_CLOCK_SKEW.as_millis()).expect("small");
        // Each instant, the state the read reports, and whether another holder's acquire and
        // an unfenced write are refused: both refuse in Live and Lapsing, neither in Lapsed.
        for (at, state) in [
            (expires - 1, LeaseState::Live),
            (expires, LeaseState::Lapsing),
            (expires + skew - 1, LeaseState::Lapsing),
            (expires + skew, LeaseState::Lapsed),
        ] {
            move_to(clock, at);
            let read = ledger.read().expect("read");
            assert_eq!(read.at, at);
            let holding = read.lease.expect("the lease is recorded in every state");
            assert_eq!(holding.state(read.at), state, "at {at}");
            let refused = state != LeaseState::Lapsed;
            let write = ledger.write(&read.token, Change::Put(record("app/run")));
            if refused {
                assert!(
                    matches!(write, Err(LedgerError::Leased { ref holder }) if holder == "alice"),
                    "at {at}: {write:?}"
                );
                assert!(
                    matches!(ledger.acquire("bob", TTL), Err(LeaseError::Held { .. })),
                    "at {at}"
                );
            } else {
                assert!(write.is_ok(), "at {at}: {write:?}");
                assert!(ledger.acquire("bob", TTL).is_ok(), "at {at}");
            }
        }
    });
}

#[test]
fn every_accepted_write_moves_the_token_even_when_it_changes_nothing() {
    on_every_ledger("tokens", |ledger| {
        let mut seen = vec![ledger.read().expect("read").token];
        for _ in 0..3 {
            put(ledger, Change::Remove(path("app/run")));
            seen.push(ledger.read().expect("read").token);
        }
        let mut unique = seen.clone();
        unique.dedup();
        assert_eq!(unique, seen);
    });
}

// ---- release records ---------------------------------------------------------------------------
// A release record is a record of the ledger like any other: the codec fills the tag, the readable
// name and the payload, and the ledger keeps them. Each test below writes through the helpers of
// `common::release`, reads the file back from a handle opened afresh, and decodes it.

/// Opens the ledger file again, as a later process does.
fn reopened(scratch: &Scratch) -> FileLedger {
    FileLedger::open(scratch.file()).expect("open again")
}

#[test]
fn publication_record_survives_reopen() {
    let scratch = Scratch::new("pub-reopen");
    let intent = publication(Some(Phase::Committing));
    let at = {
        let ledger = FileLedger::open(scratch.file()).expect("open");
        put_release(&ledger, &intent, Origin::Created, State::Intent)
    };
    let ledger = reopened(&scratch);
    assert_eq!(
        read_release(&ledger, &at),
        Some((intent.clone(), Origin::Created, State::Intent)),
        "the intent, with the phase whose effect may have landed"
    );
    // Confirmed once the read-back saw the bytes: the same path, no phase.
    let confirmed = publication(None);
    assert_eq!(
        put_release(&ledger, &confirmed, Origin::Created, State::Confirmed),
        at,
        "a publication keeps one path through its states"
    );
    drop(ledger);
    assert_eq!(
        read_release(&reopened(&scratch), &at),
        Some((confirmed, Origin::Created, State::Confirmed))
    );
    // And a coordinate found in a registry is kept as adopted.
    let found = golden_records()
        .into_iter()
        .find(|(_, origin, _)| matches!(origin, Origin::Adopted))
        .expect("an adopted publication");
    let ledger = reopened(&scratch);
    let at = put_release(&ledger, &found.0, found.1, found.2);
    drop(ledger);
    assert_eq!(read_release(&reopened(&scratch), &at), Some(found));
}

#[test]
fn run_record_payload_round_trips_through_the_file_ledger() {
    let scratch = Scratch::new("run-payload");
    let record = run();
    let at = {
        let ledger = FileLedger::open(scratch.file()).expect("open");
        put_release(&ledger, &record, Origin::Created, State::Confirmed)
    };
    let snapshot = reopened(&scratch).read().expect("read");
    let held = &snapshot.records[&at];
    assert_eq!(held.kind.as_str(), "run");
    assert_eq!(held.name, "run-7", "the name is the run id");
    assert!(held.physical.is_none() && held.deps.is_empty());
    let keys: Vec<&str> = held.outputs.keys().map(String::as_str).collect();
    let mut listed = keys_of(record.kind()).to_vec();
    listed.sort_unstable();
    assert_eq!(keys, listed, "every payload key of a run is on the list");
    assert_eq!(
        read_release(&reopened(&scratch), &at),
        Some((record, Origin::Created, State::Confirmed))
    );
}

#[test]
fn a_seal_record_round_trips_through_the_file_ledger() {
    let scratch = Scratch::new("seal");
    let record = seal();
    // A seal is created confirmed, in one write: the ledger never held it as an intent.
    let ledger = FileLedger::open(scratch.file()).expect("open");
    let before = ledger.read().expect("read").token;
    let at = put_release(&ledger, &record, Origin::Created, State::Confirmed);
    let after = ledger.read().expect("read");
    assert_ne!(after.token, before);
    assert_eq!(after.records.len(), 1);
    assert_eq!(after.records[&at].state, State::Confirmed);
    assert_eq!(
        after.records[&at].name,
        "build/linux-x86_64/0123456789abcdef"
    );
    drop(ledger);
    assert_eq!(
        read_release(&reopened(&scratch), &at),
        Some((record, Origin::Created, State::Confirmed))
    );
}

#[test]
fn a_grant_record_round_trips_and_holds_no_secret_key() {
    let scratch = Scratch::new("grant");
    let ledger = FileLedger::open(scratch.file()).expect("open");
    // The exchanges of one phase of one attempt: asked, granted, and a second one refused. Each
    // is a record of its own, and the asked one is rewritten when it is answered.
    let asked = grant(1, None);
    let first = put_release(&ledger, &asked, Origin::Created, State::Intent);
    let answered = grant(1, Some(granted()));
    assert_eq!(
        put_release(&ledger, &answered, Origin::Created, State::Confirmed),
        first
    );
    let second = put_release(
        &ledger,
        &grant(2, Some(refused())),
        Origin::Created,
        State::Confirmed,
    );
    assert_ne!(first, second);
    drop(ledger);
    let ledger = reopened(&scratch);
    assert_eq!(
        read_release(&ledger, &first),
        Some((answered, Origin::Created, State::Confirmed))
    );
    assert_eq!(
        read_release(&ledger, &second),
        Some((grant(2, Some(refused())), Origin::Created, State::Confirmed))
    );
    // The ledger is a plain text document: no key and no value of a grant is a credential.
    let text = std::fs::read_to_string(scratch.file()).expect("the file");
    for record in ledger.read().expect("read").records.values() {
        for key in record.outputs.keys() {
            let lower = key.to_ascii_lowercase();
            for secret in ["token", "secret", "password", "credential", "jwt"] {
                assert!(
                    !lower.contains(secret),
                    "the key {key} could hold a credential"
                );
            }
            assert!(
                keys_of(Kind::Grant).contains(&key.as_str()),
                "{key} is not on the list of a grant"
            );
        }
    }
    assert!(text.contains("output coordinates "));
}
