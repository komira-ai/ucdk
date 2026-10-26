// SPDX-License-Identifier: Apache-2.0
//! The ledger file the SDK tests start from, in `sdk/ledger-golden`: one whose lease another
//! holder took at a fixed instant for an hour. The SDK tests copy it into a directory of their
//! own and open a session on the copy with a clock of the session's own (`test_clock_ms`): read
//! while the hour lasts, the apply must be refused for the lease; read after the hour and the skew
//! bound, it must be let in. Here the file is held to what this crate writes, and to what it makes
//! of it.
//!
//! To change it on purpose, run
//! `cargo test -p ucdk-ledger --test golden -- --ignored write_goldens`.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::Scratch;
use ucdk_ledger::{FileLedger, LeaseError, Ledger as _, MAX_CLOCK_SKEW, ManualClock};

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
}
