// SPDX-License-Identifier: Apache-2.0
//! The records, the owner path, and the compare-and-swap document.

use std::collections::BTreeSet;

use ucdk_ledger::lease::{Epoch, Lease, LeaseError};
use ucdk_ledger::{Change, Ledger, LedgerError, MemoryLedger, Origin, OwnerPath, Record, State};
use ucdk_provider::{Kind, NodePath, PhysicalId};

fn path(text: &str) -> NodePath {
    NodePath::new(text).expect("a valid path")
}

fn record(text: &str) -> Record {
    Record::intent(path(text), Kind::service(), "name", BTreeSet::new())
}

#[test]
fn owner_path_is_every_segment_above_the_role() {
    let nested = OwnerPath::of(&path("app/db/run"));
    let names: Vec<&str> = nested
        .segments()
        .iter()
        .map(ucdk_provider::OwnerLabel::as_str)
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
    let confirmed = intent.clone().confirmed(PhysicalId::new("id-1"));
    assert_eq!(confirmed.state, State::Confirmed);
    assert_eq!(confirmed.physical, Some(PhysicalId::new("id-1")));
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
        record("app/run").confirmed(PhysicalId::new("id-1")),
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
}

/// The lease is an interface: this implementation only shows the trait can be implemented and
/// used through a trait object.
struct Fixed;

impl Lease for Fixed {
    fn acquire(&self, holder: &str) -> Result<Epoch, LeaseError> {
        if holder == "taken" {
            Err(LeaseError::Held {
                holder: "other".to_owned(),
            })
        } else {
            Ok(Epoch(1))
        }
    }

    fn release(&self, epoch: Epoch) -> Result<(), LeaseError> {
        if epoch > Epoch(1) {
            Err(LeaseError::Unavailable("unknown epoch".to_owned()))
        } else {
            Ok(())
        }
    }
}

#[test]
fn the_lease_trait_is_usable_as_an_object() {
    let lease: &dyn Lease = &Fixed;
    assert_eq!(lease.acquire("me"), Ok(Epoch(1)));
    assert!(lease.acquire("taken").is_err());
    assert_eq!(lease.release(Epoch(1)), Ok(()));
    assert!(lease.release(Epoch(2)).is_err());
}
