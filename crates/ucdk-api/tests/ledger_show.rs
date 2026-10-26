// SPDX-License-Identifier: Apache-2.0
//! `ledger_show`: one ledger document as `ucdk.api.v1` messages, with the lease judged by the
//! clock it is given, and every failure to read a ledger answered as the one `error` outcome.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ucdk_api::v1::{
    ErrorCode, ExitClass, LeaseState, LedgerRecord, LedgerShowResponse, Outcome, RecordOrigin,
    RecordState, outcome,
};
use ucdk_api::{LedgerSource, ledger_show};
use ucdk_core::{Clock, Locator, ManualClock, RecordPath, Tag};
use ucdk_ledger::{
    Change, Fence, FileLedger, Lease, LeaseError, Ledger, LedgerError, MemoryLedger, Origin,
    Record, Snapshot, State, Token,
};

/// A directory under the build's scratch area, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("api-ledger-show-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make the scratch directory");
        Self(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("ledger.ucdk")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn path(text: &str) -> RecordPath {
    RecordPath::new(text).expect("a valid path")
}

fn record(text: &str, origin: Origin, state: State) -> Record {
    Record {
        origin,
        state,
        ..Record::intent(path(text), Tag::new("service"), "a name", BTreeSet::new())
    }
}

fn put(ledger: &dyn Ledger, record: Record) {
    let token = ledger.read().expect("read").token;
    ledger
        .write(&token, Change::Put(record))
        .expect("write a record");
}

fn clock(at: u64) -> Arc<dyn Clock> {
    Arc::new(ManualClock::at(at))
}

/// The response of a call that answered, and the class it said.
fn shown(outcome: Outcome) -> LedgerShowResponse {
    assert_eq!(outcome.exit, ExitClass::Done as i32);
    match outcome.result {
        Some(outcome::Result::LedgerShow(response)) => response,
        other => panic!("not a ledger show: {other:?}"),
    }
}

/// The message of a call that answered with the ledger error, with its code and class checked.
fn ledger_error(outcome: Outcome) -> String {
    assert_eq!(outcome.exit, ExitClass::FailedNoEffect as i32);
    match outcome.result {
        Some(outcome::Result::Error(error)) => {
            assert_eq!(error.code, ErrorCode::Ledger as i32);
            error.message
        }
        other => panic!("not an error outcome: {other:?}"),
    }
}

fn open(ledger: &dyn Ledger, holder: &str) -> Outcome {
    ledger_show(&LedgerSource::Open(ledger), holder, clock(0))
}

/// A file ledger holding a lease of `ci-7` from 1,000,000 ms for a minute and one record.
fn leased_file(name: &str) -> Scratch {
    let scratch = Scratch::new(name);
    let ledger = FileLedger::open_with_clock(scratch.file(), clock(1_000_000)).expect("open");
    put(
        &ledger,
        record("app/run", Origin::Created, State::Confirmed),
    );
    ledger
        .acquire("ci-7", Duration::from_secs(60))
        .expect("acquire");
    scratch
}

#[test]
fn every_record_state_and_origin_is_mapped() {
    let ledger = MemoryLedger::new();
    let wanted = [
        ("a/intent-created", Origin::Created, State::Intent, 1, 1),
        (
            "a/confirmed-created",
            Origin::Created,
            State::Confirmed,
            1,
            2,
        ),
        ("a/reaping-created", Origin::Created, State::Reaping, 1, 3),
        ("a/intent-adopted", Origin::Adopted, State::Intent, 2, 1),
        (
            "a/confirmed-adopted",
            Origin::Adopted,
            State::Confirmed,
            2,
            2,
        ),
        ("a/reaping-adopted", Origin::Adopted, State::Reaping, 2, 3),
    ];
    for (name, origin, state, _, _) in wanted {
        put(&ledger, record(name, origin, state));
    }
    let response = shown(open(&ledger, "me"));
    assert_eq!(response.records.len(), wanted.len());
    for (name, _, _, origin, state) in wanted {
        let found = response
            .records
            .iter()
            .find(|r| r.path == name)
            .expect("a record");
        assert_eq!(found.origin, origin, "{name}");
        assert_eq!(found.state, state, "{name}");
    }
    // The wire numbers are the enums' own.
    assert_eq!(RecordOrigin::Created as i32, 1);
    assert_eq!(RecordOrigin::Adopted as i32, 2);
    assert_eq!(RecordState::Intent as i32, 1);
    assert_eq!(RecordState::Reaping as i32, 3);
}

#[test]
fn a_record_is_shown_with_every_one_of_its_fields() {
    let ledger = MemoryLedger::new();
    let full = Record {
        physical: Some(Locator::new("run-1")),
        deps: BTreeSet::from([path("app/db"), path("app/cache")]),
        outputs: BTreeMap::from([("HOST".to_owned(), "run-host".to_owned())]),
        ..record("app/run", Origin::Created, State::Confirmed)
    };
    put(&ledger, full);
    put(&ledger, record("app/bare", Origin::Created, State::Intent));
    let response = shown(open(&ledger, "me"));
    let by_path = |p: &str| -> &LedgerRecord {
        response
            .records
            .iter()
            .find(|r| r.path == p)
            .expect("a record")
    };
    let run = by_path("app/run");
    assert_eq!(
        (run.kind.as_str(), run.name.as_str()),
        ("service", "a name")
    );
    assert_eq!(run.physical, "run-1");
    assert_eq!(
        run.deps,
        ["app/cache", "app/db"],
        "dependencies in path order"
    );
    assert_eq!(run.outputs.len(), 1);
    assert_eq!(run.outputs[0].key, "HOST");
    assert_eq!(run.outputs[0].value, "run-host");
    // A record with no physical id shows an empty one.
    assert_eq!(by_path("app/bare").physical, "");
    assert_eq!(response.token, "v2");
}

#[test]
fn outputs_are_pairs_in_key_order_and_records_in_path_order() {
    let ledger = MemoryLedger::new();
    for name in ["b/run", "a/run", "a/db"] {
        let outputs = ["z", "a", "m"]
            .iter()
            .map(|k| ((*k).to_owned(), format!("value-{k}")))
            .collect();
        put(
            &ledger,
            Record {
                outputs,
                ..record(name, Origin::Created, State::Confirmed)
            },
        );
    }
    let response = shown(open(&ledger, "me"));
    let paths: Vec<&str> = response.records.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["a/db", "a/run", "b/run"]);
    for record in &response.records {
        let pairs: Vec<(&str, &str)> = record
            .outputs
            .iter()
            .map(|p| (p.key.as_str(), p.value.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [("a", "value-a"), ("m", "value-m"), ("z", "value-z")]
        );
    }
}

#[test]
fn an_absent_lease_is_absent() {
    let ledger = MemoryLedger::new();
    let none = shown(open(&ledger, "me"));
    assert_eq!(none.lease, None);
    assert_eq!((none.token.as_str(), none.fence), ("v0", 0));
    // A lease that was given back is absent too, and the fence stays.
    let lease = ledger
        .acquire("ci-7", Duration::from_secs(60))
        .expect("acquire");
    ledger.release(lease.fence).expect("release");
    let released = shown(open(&ledger, "me"));
    assert_eq!(released.lease, None);
    assert_eq!(released.fence, 1);
}

#[test]
fn a_lease_is_live_lapsing_or_lapsed_by_the_clock_it_was_given() {
    let scratch = leased_file("states");
    let expires = 1_060_000;
    let skew = 5_000;
    for (at, state) in [
        (expires - 1, LeaseState::Live),
        (expires, LeaseState::Lapsing),
        (expires + skew - 1, LeaseState::Lapsing),
        (expires + skew, LeaseState::Lapsed),
    ] {
        let response = shown(ledger_show(
            &LedgerSource::File(&scratch.file()),
            "me",
            clock(at),
        ));
        let lease = response.lease.expect("a lease");
        assert_eq!(lease.state, state as i32, "at {at}");
        assert_eq!((lease.holder.as_str(), lease.expires_ms), ("ci-7", expires));
        assert_eq!(response.fence, 1);
    }
}

#[test]
fn yours_compares_the_holder() {
    let scratch = leased_file("yours");
    let yours = |holder: &str| {
        shown(ledger_show(
            &LedgerSource::File(&scratch.file()),
            holder,
            clock(1_000_001),
        ))
        .lease
        .expect("a lease")
        .yours
    };
    assert!(yours("ci-7"));
    assert!(!yours("ci-8"));
    assert!(!yours(""), "no name is no lease's holder");
}

#[test]
fn a_call_changes_no_file_and_creates_none() {
    let scratch = leased_file("no_change");
    let listing = || -> BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(scratch.file().parent().expect("a directory"))
            .expect("list")
            .map(|e| {
                let e = e.expect("entry");
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).expect("read"),
                )
            })
            .collect()
    };
    std::fs::remove_file(scratch.file().with_extension("ucdk.lock")).expect("remove the lock file");
    let before = listing();
    let _ = shown(ledger_show(
        &LedgerSource::File(&scratch.file()),
        "me",
        clock(1_000_001),
    ));
    assert_eq!(listing(), before, "names and bytes are as they were");
}

#[test]
fn a_corrupt_file_is_the_ledger_error() {
    let scratch = Scratch::new("corrupt");
    std::fs::write(scratch.file(), "this is not a ledger\n").expect("write");
    let message = ledger_error(ledger_show(
        &LedgerSource::File(&scratch.file()),
        "me",
        clock(0),
    ));
    assert!(
        message.contains("corrupt") || message.contains("not a ucdk ledger"),
        "{message}"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.file()).expect("read"),
        "this is not a ledger\n",
        "the file is left as it was"
    );
}

#[test]
fn a_newer_major_is_named_in_the_message() {
    let scratch = Scratch::new("newer");
    // A whole document of a major this build does not read: only the header and the end line.
    let body = "ucdk-ledger 9.0\nversion 1\n";
    let sum = body.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    std::fs::write(scratch.file(), format!("{body}end 0 {sum:016x}\n")).expect("write");
    let message = ledger_error(ledger_show(
        &LedgerSource::File(&scratch.file()),
        "me",
        clock(0),
    ));
    assert!(message.contains("version 9"), "{message}");
    assert!(message.contains("newer"), "{message}");
}

#[test]
fn a_missing_file_is_the_ledger_error() {
    let scratch = Scratch::new("missing");
    let message = ledger_error(ledger_show(
        &LedgerSource::File(&scratch.file()),
        "me",
        clock(0),
    ));
    assert!(message.contains("does not exist"), "{message}");
    assert!(!scratch.file().exists(), "an empty ledger was not made");
}

/// A ledger that cannot be read.
struct Down;

impl Ledger for Down {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        Err(LedgerError::Unavailable("reads are down".to_owned()))
    }

    fn write(&self, _: &Token, _: Change) -> Result<Token, LedgerError> {
        unreachable!("a show writes nothing")
    }

    fn write_fenced(&self, _: Fence, _: &Token, _: Change) -> Result<Token, LedgerError> {
        unreachable!("a show writes nothing")
    }

    fn acquire(&self, _: &str, _: Duration) -> Result<Lease, LeaseError> {
        unreachable!("a show takes no lease")
    }

    fn renew(&self, _: Fence, _: Duration) -> Result<Lease, LedgerError> {
        unreachable!("a show takes no lease")
    }

    fn release(&self, _: Fence) -> Result<(), LedgerError> {
        unreachable!("a show takes no lease")
    }
}

#[test]
fn a_ledger_whose_read_fails_is_the_ledger_error() {
    let message = ledger_error(open(&Down, "me"));
    assert!(message.contains("reads are down"), "{message}");
}
