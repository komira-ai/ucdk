// SPDX-License-Identifier: Apache-2.0
#![cfg(feature = "test-clock")]
//! The ledger lease through the ABI, on a session whose clock the test fixes
//! (`SessionConfig.test_clock_ms`, which only a build with the `test-clock` feature honours; the
//! default build refuses it, see `test_clock_default.rs`).

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{call, file_config, open};
use prost::Message as _;
use ucdk_api::v1::{ExitClass, LeaseState, LeaseView, LedgerShowRequest, Outcome, outcome};
use ucdk_capi::{OK, ucdk_ledger_show, ucdk_session_close};
use ucdk_ledger::{Clock, FileLedger, Ledger as _, ManualClock};

/// When `ci-7` took the lease in these tests, and for how long; and the times a session sees.
const GRANTED_MS: u64 = 1_000_000;
const LEASE: Duration = Duration::from_secs(3600);
const EXPIRES_MS: u64 = GRANTED_MS + 3_600_000;
const SKEW_MS: u64 = 5_000;

/// A file ledger in a fresh directory of its own that `holder` leased for `ttl` on `clock`.
fn file_ledger_leased_by(
    scratch: &common::Scratch,
    holder: &str,
    ttl: Duration,
    clock: Arc<dyn Clock>,
) -> String {
    let location = scratch.file();
    let ledger = FileLedger::open_with_clock(&location, clock).expect("open");
    ledger.acquire(holder, ttl).expect("acquire");
    drop(ledger);
    // The lock file is the writer's, not what the sessions under test do.
    std::fs::remove_file(scratch.dir().join("ledger.ucdk.lock")).expect("remove the lock file");
    location.to_string_lossy().into_owned()
}

/// What a session on the ledger at `location`, holder `holder` and the clock fixed at `now_ms`,
/// shows of the lease.
fn lease_seen(location: &str, holder: &str, now_ms: u64) -> LeaseView {
    let mut config = file_config(std::path::Path::new(location));
    config.holder = holder.to_owned();
    config.test_clock_ms = now_ms;
    let opened = open(&config);
    assert_eq!(opened.status, OK);
    let reply = call(
        ucdk_ledger_show,
        opened.session,
        &LedgerShowRequest {}.encode_to_vec(),
    );
    assert_eq!(reply.status, OK);
    ucdk_session_close(opened.session);
    let answer = Outcome::decode(reply.bytes.as_slice()).expect("an outcome");
    assert_eq!(answer.exit, ExitClass::Done as i32, "{answer:?}");
    let Some(outcome::Result::LedgerShow(response)) = answer.result else {
        panic!("not a ledger show: {answer:?}");
    };
    response.lease.expect("the lease of the file")
}

/// Proves: a ledger show reports another holder's lease as live before it expires, lapsing from
/// the expiry until the skew bound after it, and lapsed after, each by the clock the session was
/// opened with, and never reports it as the session's own. Catches a show that reads the wall
/// clock, ignores `test_clock_ms`, or draws the lapsing line somewhere else than the ledger does.
#[test]
fn ledger_show_reports_another_holders_lease_live_then_lapsing_then_lapsed_by_the_session_clock() {
    let scratch = common::Scratch::new("clock-states");
    let leased = file_ledger_leased_by(
        &scratch,
        "ci-7",
        LEASE,
        Arc::new(ManualClock::at(GRANTED_MS)),
    );
    for (now_ms, state) in [
        (GRANTED_MS + 1_000, LeaseState::Live),
        (EXPIRES_MS - 1, LeaseState::Live),
        (EXPIRES_MS, LeaseState::Lapsing),
        (EXPIRES_MS + SKEW_MS - 1, LeaseState::Lapsing),
        (EXPIRES_MS + SKEW_MS, LeaseState::Lapsed),
    ] {
        let lease = lease_seen(&leased, "ci-8", now_ms);
        assert_eq!(lease.state, state as i32, "at {now_ms}");
        assert_eq!(lease.holder, "ci-7");
        assert_eq!(lease.expires_ms, EXPIRES_MS);
        assert!(!lease.yours, "another holder's lease is not the session's");
    }
}

/// Proves: a session whose config names the holder of the lease sees it as its own (`yours`), at
/// every state, and a session with another name or none does not. Catches a config holder that is
/// ignored by the show.
#[test]
fn a_session_named_as_the_holder_of_a_lease_sees_it_as_its_own() {
    let scratch = common::Scratch::new("clock-yours");
    let leased = file_ledger_leased_by(
        &scratch,
        "ci-7",
        LEASE,
        Arc::new(ManualClock::at(GRANTED_MS)),
    );
    for now_ms in [GRANTED_MS + 1_000, EXPIRES_MS, EXPIRES_MS + SKEW_MS] {
        assert!(lease_seen(&leased, "ci-7", now_ms).yours, "at {now_ms}");
        for other in ["ci-8", ""] {
            assert!(
                !lease_seen(&leased, other, now_ms).yours,
                "{other:?} at {now_ms}"
            );
        }
    }
}
