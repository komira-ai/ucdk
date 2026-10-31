// SPDX-License-Identifier: Apache-2.0
//! The ledger file the SDK tests start from, in `sdk/ledger-golden`: one whose lease another
//! holder took at a fixed instant for an hour. The SDK tests copy it into a directory of their
//! own and open a session on the copy with a clock of the session's own (`test_clock_ms`): read
//! while the hour lasts, the apply must be refused for the lease; read after the hour and the skew
//! bound, it must be let in. Here the file is held to what this crate writes, and to what it makes
//! of it.
//!
//! A second file, `records.ucdk`, holds records of every state and origin and no lease. It is
//! the byte-level promise for the record lines: what this crate wrote for them is what it must
//! keep writing, and a file with these lines must keep reading to the same records.
//!
//! A third, `release-records.ucdk`, holds records the release codec wrote (a run, publications,
//! a seal and grants) and no lease: the release side adds no field and no format to the ledger,
//! and the file is what shows it.
//!
//! To change them on purpose, run
//! `cargo test -p ucdk-ledger --test golden -- --ignored write_goldens`.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::release::{golden_records, ledger_record, read_release};
use common::{Scratch, path};
use ucdk_core::{Locator, RecordPath, Tag};
use ucdk_ledger::{
    Change, FileLedger, LeaseError, Ledger as _, MAX_CLOCK_SKEW, ManualClock, Origin, OwnerPath,
    Record, State,
};

/// When `ci-7` took the lease, in milliseconds on the clock the file was written with.
const GRANTED: u64 = 1_000_000;
const HOUR: Duration = Duration::from_secs(3600);

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdk/ledger-golden")
}

/// What `name` holds when this crate writes it: `ci-7` takes the lease for an hour at `at`.
fn written(name: &str, at: u64) -> Vec<u8> {
    let scratch = Scratch::new(name);
    let ledger =
        FileLedger::open_with_clock(scratch.file(), Arc::new(ManualClock::at(at))).expect("open");
    ledger.acquire("ci-7", HOUR).expect("acquire");
    drop(ledger);
    std::fs::read(scratch.file()).expect("the ledger file")
}

/// The file, written in a scratch directory named for `tag` (tests run side by side).
fn golden(tag: &str) -> Vec<u8> {
    written(&format!("golden-leased-{tag}"), GRANTED)
}

#[test]
fn the_ledger_golden_is_the_file_this_crate_writes() {
    let on_disk = std::fs::read(dir().join("leased.ucdk")).expect("leased.ucdk");
    assert_eq!(
        on_disk,
        golden("check"),
        "leased.ucdk differs from what is written now"
    );
}

#[test]
fn the_golden_is_held_by_ci_7_while_the_hour_lasts_and_can_be_taken_after_it() {
    let scratch = Scratch::new("golden-read");
    std::fs::copy(dir().join("leased.ucdk"), scratch.file()).expect("copy");
    let clock = ManualClock::at(GRANTED + 1_000);
    let ledger =
        FileLedger::open_with_clock(scratch.file(), Arc::new(clock.clone())).expect("open");
    let held = ledger.acquire("someone", HOUR).expect_err("held");
    assert!(matches!(held, LeaseError::Held { ref holder, .. } if holder == "ci-7"));
    clock.advance(HOUR + MAX_CLOCK_SKEW);
    assert_eq!(
        ledger
            .acquire("someone", HOUR)
            .expect("lapsed")
            .fence
            .value(),
        2
    );
}

#[test]
#[ignore = "writes the golden file; run on purpose"]
fn write_goldens() {
    std::fs::create_dir_all(dir()).expect("create the directory");
    std::fs::write(dir().join("leased.ucdk"), golden("write")).expect("write a golden");
    std::fs::write(dir().join("records.ucdk"), written_records("write")).expect("write a golden");
    std::fs::write(
        dir().join("release-records.ucdk"),
        written_release_records("write"),
    )
    .expect("write a golden");
}

/// The records the second golden holds, one for each state and each origin, with every line a
/// record can carry: a kind, a name, a physical id, dependencies and outputs. The values are
/// plain words so that the text of the file is readable; the last record holds an output that
/// needs escaping and an empty one.
fn sample_records() -> Vec<Record> {
    let record = |text: &str, kind: &str, name: &str, origin: Origin, state: State| Record {
        owner: OwnerPath::of(&path(text)),
        path: path(text),
        kind: Tag::new(kind),
        name: name.to_owned(),
        origin,
        state,
        physical: None,
        deps: BTreeSet::new(),
        outputs: BTreeMap::new(),
    };
    let outputs = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>()
    };
    vec![
        Record {
            outputs: outputs(&[("HOST", "db-host"), ("PORT", "5432")]),
            ..record(
                "app/db",
                "service",
                "app-db",
                Origin::Created,
                State::Confirmed,
            )
            .confirmed(Locator::new("db-1"))
        },
        Record {
            deps: BTreeSet::from([path("app/db")]),
            outputs: outputs(&[("URL", "run-url")]),
            ..record(
                "app/run",
                "service",
                "app-run",
                Origin::Created,
                State::Confirmed,
            )
            .confirmed(Locator::new("run-1"))
        },
        record(
            "app/pending",
            "job",
            "app-pending",
            Origin::Created,
            State::Intent,
        ),
        record("app/old", "job", "app-old", Origin::Created, State::Reaping)
            .confirmed(Locator::new("old-1"))
            .in_state(State::Reaping),
        Record {
            outputs: outputs(&[("EMPTY", ""), ("NOTE", "two words")]),
            ..record(
                "shared/found",
                "volume",
                "shared-found",
                Origin::Adopted,
                State::Confirmed,
            )
            .confirmed(Locator::new("found-1"))
        },
    ]
}

/// What the second golden holds when this crate writes it: the records of [`sample_records`],
/// no lease, under the fixed clock.
fn written_records(tag: &str) -> Vec<u8> {
    let scratch = Scratch::new(&format!("golden-records-{tag}"));
    let ledger = FileLedger::open_with_clock(scratch.file(), Arc::new(ManualClock::at(GRANTED)))
        .expect("open");
    for record in sample_records() {
        let token = ledger.read().expect("read").token;
        ledger.write(&token, Change::Put(record)).expect("write");
    }
    drop(ledger);
    std::fs::read(scratch.file()).expect("the ledger file")
}

#[test]
fn the_records_golden_is_the_file_this_crate_writes() {
    let on_disk = std::fs::read(dir().join("records.ucdk")).expect("records.ucdk");
    assert_eq!(
        on_disk,
        written_records("check"),
        "records.ucdk differs from what is written now"
    );
}

#[test]
fn the_records_golden_reads_back_to_the_same_records() {
    let scratch = Scratch::new("golden-records-read");
    std::fs::copy(dir().join("records.ucdk"), scratch.file()).expect("copy");
    let ledger = FileLedger::open(scratch.file()).expect("open");
    let read = ledger.read().expect("read");
    let wanted: BTreeMap<RecordPath, Record> = sample_records()
        .into_iter()
        .map(|r| (r.path.clone(), r))
        .collect();
    assert_eq!(read.records, wanted);
    let states: BTreeSet<_> = read
        .records
        .values()
        .map(|r| format!("{:?}", r.state))
        .collect();
    let origins: BTreeSet<_> = read
        .records
        .values()
        .map(|r| format!("{:?}", r.origin))
        .collect();
    assert_eq!(states.len(), 3, "every state is in the golden");
    assert_eq!(origins.len(), 2, "every origin is in the golden");
}

/// What the third golden holds when this crate writes it: the records of [`golden_records`] as
/// the release codec fills them, one write each, no lease, under the fixed clock.
fn written_release_records(tag: &str) -> Vec<u8> {
    let scratch = Scratch::new(&format!("golden-release-{tag}"));
    let ledger = FileLedger::open_with_clock(scratch.file(), Arc::new(ManualClock::at(GRANTED)))
        .expect("open");
    for (release, origin, state) in golden_records() {
        let token = ledger.read().expect("read").token;
        ledger
            .write(&token, Change::Put(ledger_record(&release, origin, state)))
            .expect("write");
    }
    drop(ledger);
    std::fs::read(scratch.file()).expect("the ledger file")
}

#[test]
fn a_release_records_golden_is_the_file_the_codec_and_this_crate_write() {
    let on_disk =
        std::fs::read_to_string(dir().join("release-records.ucdk")).expect("release-records.ucdk");
    let written = String::from_utf8(written_release_records("check")).expect("text");
    assert_eq!(
        on_disk, written,
        "release-records.ucdk differs from what is written now"
    );
}

#[test]
fn a_release_records_golden_reads_back() {
    let scratch = Scratch::new("golden-release-read");
    std::fs::copy(dir().join("release-records.ucdk"), scratch.file()).expect("copy");
    let ledger = FileLedger::open(scratch.file()).expect("open");
    let read = ledger.read().expect("read");
    assert!(read.lease.is_none(), "the golden holds no lease");
    let wanted = golden_records();
    assert_eq!(read.records.len(), wanted.len());
    for (release, origin, state) in wanted {
        let at = ledger_record(&release, origin, state).path;
        assert_eq!(
            read_release(&ledger, &at),
            Some((release, origin, state)),
            "{at}"
        );
    }
    let kinds: BTreeSet<&str> = read.records.values().map(|r| r.kind.as_str()).collect();
    assert_eq!(
        kinds,
        BTreeSet::from(["grant", "publication", "run", "seal"]),
        "the golden holds the four kinds this change writes"
    );
    for record in read.records.values() {
        let owner = record.path.as_str().split('/').next().expect("a segment");
        assert_eq!(owner, record.kind.as_str(), "the owner is the tag");
        assert!(record.physical.is_none() && record.deps.is_empty());
    }
}
