// SPDX-License-Identifier: Apache-2.0
//! The records, the owner path, and the compare-and-swap document.

use std::collections::BTreeSet;

use ucdk_core::{Locator, RecordPath, Tag};
use ucdk_ledger::{Change, Ledger, LedgerError, MemoryLedger, Origin, OwnerPath, Record, State};

fn path(text: &str) -> RecordPath {
    RecordPath::new(text).expect("a valid path")
}

fn record(text: &str) -> Record {
    Record::intent(path(text), Tag::new("service"), "name", BTreeSet::new())
}

#[test]
fn owner_path_is_every_segment_above_the_role() {
    let nested = OwnerPath::of(&path("app/db/run"));
    let names: Vec<&str> = nested
        .segments()
        .iter()
        .map(ucdk_core::OwnerLabel::as_str)
        .collect();
    assert_eq!(names, ["app", "db"]);
    assert_eq!(nested.top().as_str(), "app");
    assert_eq!(nested.to_string(), "app/db");
    assert_eq!(OwnerPath::of(&path("app/run")).to_string(), "app");
}

#[test]
fn a_path_of_one_segment_is_its_own_owner() {
    let single = OwnerPath::of(&path("app"));
    assert_eq!(single.to_string(), "app");
    assert_eq!(single.top().as_str(), "app");
}

#[test]
fn a_first_intent_is_created_with_no_physical_id() {
    let r = record("app/run");
    assert_eq!(r.state, State::Intent);
    assert_eq!(r.origin, Origin::Created);
    assert_eq!(r.physical, None);
    assert_eq!(r.owner.to_string(), "app");
    assert_eq!(r.name, "name");
}

#[test]
fn confirming_records_the_physical_id_and_changes_nothing_else() {
    let intent = record("app/run");
    let confirmed = intent.clone().confirmed(Locator::new("id-1"));
    assert_eq!(confirmed.state, State::Confirmed);
    assert_eq!(confirmed.physical, Some(Locator::new("id-1")));
    assert_eq!(confirmed.path, intent.path);
    assert_eq!(confirmed.owner, intent.owner);
    let reaping = confirmed.clone().in_state(State::Reaping);
    assert_eq!(reaping.state, State::Reaping);
    assert_eq!(reaping.physical, confirmed.physical);
}

#[test]
fn a_new_ledger_is_empty() {
    let snapshot = MemoryLedger::new().read().expect("read");
    assert!(snapshot.records.is_empty());
    assert_eq!(snapshot.token.to_string(), "v0");
}

#[test]
fn a_write_with_the_current_token_lands_and_returns_the_next() {
    let ledger = MemoryLedger::new();
    let first = ledger.read().expect("read").token;
    let second = ledger
        .write(&first, Change::Put(record("app/run")))
        .expect("write");
    assert_ne!(first, second);
    let snapshot = ledger.read().expect("read");
    assert_eq!(snapshot.token, second);
    assert_eq!(snapshot.records.len(), 1);
}

#[test]
fn a_write_with_a_stale_token_is_refused_and_changes_nothing() {
    let ledger = MemoryLedger::new();
    let old = ledger.read().expect("read").token;
    ledger
        .write(&old, Change::Put(record("app/run")))
        .expect("write");
    let result = ledger.write(&old, Change::Put(record("db/run")));
    assert_eq!(result, Err(LedgerError::Stale));
    assert_eq!(ledger.read().expect("read").records.len(), 1);
}

#[test]
fn put_replaces_the_record_of_a_path_and_remove_drops_it() {
    let ledger = MemoryLedger::new();
    let put = |change: Change| {
        let token = ledger.read().expect("read").token;
        ledger.write(&token, change).expect("write");
    };
    put(Change::Put(record("app/run")));
    put(Change::Put(
        record("app/run").confirmed(Locator::new("id-1")),
    ));
    let records = ledger.read().expect("read").records;
    assert_eq!(records.len(), 1);
    assert_eq!(records[&path("app/run")].state, State::Confirmed);
    put(Change::Remove(path("app/run")));
    assert!(ledger.read().expect("read").records.is_empty());
    // Removing a path with no record is not an error.
    put(Change::Remove(path("app/run")));
}

#[test]
fn a_clone_shares_the_document() {
    let ledger = MemoryLedger::new();
    let other = ledger.clone();
    let token = ledger.read().expect("read").token;
    ledger
        .write(&token, Change::Put(record("app/run")))
        .expect("write");
    assert_eq!(other.read().expect("read").records.len(), 1);
}

#[test]
fn errors_read_as_sentences() {
    assert_eq!(
        LedgerError::Stale.to_string(),
        "the ledger changed since it was read"
    );
    assert_eq!(
        LedgerError::Unavailable("down".to_owned()).to_string(),
        "the ledger is unavailable: down"
    );
    assert_eq!(
        LedgerError::ReadOnly.to_string(),
        "the ledger was opened read-only and cannot be written"
    );
    assert_eq!(
        ucdk_ledger::LeaseError::ReadOnly.to_string(),
        "the ledger was opened read-only: no lease is granted"
    );
    assert_eq!(
        ucdk_ledger::LeaseError::Changed.to_string(),
        "the ledger changed since it was read, or a lease was granted since: no lease is granted"
    );
}

#[test]
fn a_holding_is_live_then_lapsing_then_lapsed_at_the_edges() {
    use ucdk_ledger::{Fence, Holding, LeaseState, MAX_CLOCK_SKEW};
    let holding = Holding {
        holder: "ci-7".to_owned(),
        fence: Fence::new(3),
        expires_at: 10_000,
    };
    let skew = u64::try_from(MAX_CLOCK_SKEW.as_millis()).expect("small");
    assert_eq!(holding.state(0), LeaseState::Live);
    assert_eq!(holding.state(9_999), LeaseState::Live);
    assert_eq!(holding.state(10_000), LeaseState::Lapsing);
    assert_eq!(holding.state(10_000 + skew - 1), LeaseState::Lapsing);
    assert_eq!(holding.state(10_000 + skew), LeaseState::Lapsed);
    assert_eq!(holding.state(u64::MAX), LeaseState::Lapsed);
    let never = Holding {
        expires_at: u64::MAX,
        ..holding
    };
    assert_eq!(never.state(u64::MAX - 1), LeaseState::Live);
    assert_eq!(
        never.state(u64::MAX),
        LeaseState::Lapsed,
        "the sum saturates"
    );
}
