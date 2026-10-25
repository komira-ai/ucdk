// SPDX-License-Identifier: Apache-2.0
//! The compare-and-swap contract, run against every ledger: each test below is run once on the
//! in-memory ledger and once on the file ledger, so the two cannot drift apart.

mod common;

use common::{Scratch, confirmed, path, put, record};
use ucdk_ledger::{Change, FileLedger, Ledger, LedgerError, MemoryLedger, State};

/// Runs `check` on a fresh ledger of each kind.
fn on_every_ledger(name: &str, check: impl Fn(&dyn Ledger)) {
    check(&MemoryLedger::new());
    let scratch = Scratch::new(name);
    check(&FileLedger::open(scratch.file()).expect("open"));
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
