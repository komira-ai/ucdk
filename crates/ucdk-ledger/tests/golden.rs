// SPDX-License-Identifier: Apache-2.0
//! The ledger files the SDK tests start from, in `sdk/ledger-golden`: one whose lease another
//! holder has for a very long time, and one whose lease lapsed long ago. The SDK tests copy them
//! into a directory of their own, open a session on the copy and apply: the first must be refused
//! for the lease and the second let in. Here the files are held to what this crate writes, and to
//! what it makes of them.
//!
//! To change them on purpose, run
//! `cargo test -p ucdk-ledger --test golden -- --ignored write_goldens`.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::Scratch;
use ucdk_ledger::{FileLedger, LeaseError, Ledger as _, ManualClock};

/// Milliseconds since the epoch of the first instant of the year 2100.
const FAR_FUTURE: u64 = 4_102_444_800_000;
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

/// Both files, written in scratch directories named for `tag` (tests run side by side).
fn goldens(tag: &str) -> [(&'static str, Vec<u8>); 2] {
    [
        (
            "leased.ucdk",
            written(&format!("golden-leased-{tag}"), FAR_FUTURE - 3_600_000),
        ),
        (
            "lapsed.ucdk",
            written(&format!("golden-lapsed-{tag}"), 1_000),
        ),
    ]
}

#[test]
fn the_ledger_goldens_are_the_files_this_crate_writes() {
    for (name, bytes) in goldens("check") {
        let on_disk = std::fs::read(dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(on_disk, bytes, "{name} differs from what is written now");
    }
}

#[test]
fn the_leased_golden_is_held_by_ci_7_and_the_lapsed_one_can_be_taken() {
    let scratch = Scratch::new("golden-read");
    for (name, _) in goldens("read") {
        std::fs::copy(dir().join(name), scratch.dir().join(name)).expect("copy");
    }
    let leased = FileLedger::open(scratch.dir().join("leased.ucdk")).expect("open");
    let held = leased.acquire("someone", HOUR).expect_err("held");
    assert!(matches!(held, LeaseError::Held { ref holder, .. } if holder == "ci-7"));
    let lapsed = FileLedger::open(scratch.dir().join("lapsed.ucdk")).expect("open");
    assert_eq!(
        lapsed
            .acquire("someone", HOUR)
            .expect("lapsed")
            .fence
            .value(),
        2
    );
}

#[test]
#[ignore = "writes the golden files; run on purpose"]
fn write_goldens() {
    std::fs::create_dir_all(dir()).expect("create the directory");
    for (name, bytes) in goldens("write") {
        std::fs::write(dir().join(name), bytes).expect("write a golden");
    }
}
