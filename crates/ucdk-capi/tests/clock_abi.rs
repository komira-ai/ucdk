// SPDX-License-Identifier: Apache-2.0
#![cfg(feature = "test-clock")]
//! The ledger lease through the ABI, on a session whose clock the test fixes
//! (`SessionConfig.test_clock_ms`, which only a build with the `test-clock` feature honours; the
//! default build refuses it, see `test_clock_default.rs`).

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{call, fake_config, list_of, open, service_of};
use prost::Message as _;
use ucdk_api::v1::{self, ApplyRequest, ExitClass, Mode, Outcome, PlanRequest, outcome};
use ucdk_capi::{OK, ucdk_apply, ucdk_plan, ucdk_session_close};
use ucdk_ledger::{Clock, FileLedger, Ledger as _, ManualClock};
use ucdk_proto::v1::ResourceList;

fn planned_hash(answer: &Outcome) -> String {
    match &answer.result {
        Some(outcome::Result::Planned(plan)) => plan.hash.clone(),
        other => panic!("not a plan: {other:?}"),
    }
}

fn close(session: *mut ucdk_capi::SessionHandle) {
    ucdk_session_close(session);
}

/// A file ledger in a fresh directory of its own that `holder` leased for `ttl` before the
/// session opens it, on `clock`.
fn file_ledger_leased_by(name: &str, holder: &str, ttl: Duration, clock: Arc<dyn Clock>) -> String {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let location = dir.join("ledger").to_string_lossy().into_owned();
    let ledger = FileLedger::open_with_clock(&location, clock).expect("open");
    ledger.acquire(holder, ttl).expect("acquire");
    location
}

/// The outcome of planning `list` and applying it under that plan's hash through a session on
/// the file ledger at `location`.
fn apply_through_the_abi(location: &str, list: &ResourceList, now_ms: u64) -> Outcome {
    apply_through_the_abi_as(location, list, "", now_ms)
}

/// When `ci-7` took the lease in these tests, and for how long; and the times the session sees.
const GRANTED_MS: u64 = 1_000_000;
const LEASE: Duration = Duration::from_secs(3600);
const WHILE_HELD_MS: u64 = GRANTED_MS + 1_000;
const AFTER_LAPSE_MS: u64 = GRANTED_MS + 3_600_000 + 5_000;

/// [`apply_through_the_abi`] in a session whose runs take the lease as `holder` (empty: unnamed).
fn apply_through_the_abi_as(
    location: &str,
    list: &ResourceList,
    holder: &str,
    now_ms: u64,
) -> Outcome {
    let mut config = fake_config();
    config.holder = holder.to_owned();
    config.test_clock_ms = now_ms;
    config.ledger_kind = "file".to_owned();
    config.ledger_location = location.to_owned();
    let opened = open(&config);
    assert_eq!(opened.status, OK);
    let session = opened.session;
    let plan = PlanRequest {
        resources: Some(list.clone()),
        mode: Mode::Apply as i32,
    };
    let planned = call(ucdk_plan, session, &plan.encode_to_vec());
    let hash = planned_hash(&Outcome::decode(planned.bytes.as_slice()).expect("an outcome"));
    let apply = ApplyRequest {
        resources: Some(list.clone()),
        plan_hash: hash,
        rollback: false,
    };
    let reply = call(ucdk_apply, session, &apply.encode_to_vec());
    assert_eq!(reply.status, OK);
    close(session);
    Outcome::decode(reply.bytes.as_slice()).expect("an outcome")
}

/// Proves: an apply through the ABI on a ledger whose lease another holder has is refused as
/// data (the refusal names the holder, the exit class is REFUSED), the same as the command
/// line's, and creates nothing; once that holder's lease has lapsed, the same call is let in.
/// Catches an ABI that reports the lease as a status code, or one that ignores a lease on file.
#[test]
fn an_apply_on_a_ledger_leased_to_another_holder_is_refused_as_data_until_it_lapses() {
    let list = list_of(vec![service_of("a-svc", 80)]);
    let live = file_ledger_leased_by(
        "abi-leased",
        "ci-7",
        LEASE,
        Arc::new(ManualClock::at(GRANTED_MS)),
    );
    let answer = apply_through_the_abi(&live, &list, WHILE_HELD_MS);
    assert_eq!(answer.exit, ExitClass::Refused as i32, "{answer:?}");
    let Some(outcome::Result::Refused(why)) = answer.result else {
        panic!("not refused: {answer:?}");
    };
    let Some(v1::refusal::Reason::LeaseHeld(held)) = why.reason else {
        panic!("not a lease refusal: {why:?}");
    };
    assert_eq!(held.holder, "ci-7");
    assert_eq!(held.remaining_ms, 3_600_000 - 1_000 + 5_000);

    // The session's clock reads past the lease and the skew bound: the holder is taken over.
    let dead = file_ledger_leased_by(
        "abi-lapsed",
        "ci-7",
        LEASE,
        Arc::new(ManualClock::at(GRANTED_MS)),
    );
    let answer = apply_through_the_abi(&dead, &list, AFTER_LAPSE_MS);
    assert_eq!(answer.exit, ExitClass::Done as i32, "{answer:?}");
}

/// Proves: a session whose config names the holder a lease is in takes that lease again at once
/// and applies, where an unnamed session, or one with another name, is refused. Catches a
/// config holder that is ignored (each run then gets a name no earlier run had).
#[test]
fn a_session_named_as_the_holder_of_a_lease_reclaims_it() {
    let list = list_of(vec![service_of("a-svc", 80)]);
    let leased = file_ledger_leased_by(
        "abi-reclaim",
        "ci-7",
        LEASE,
        Arc::new(ManualClock::at(GRANTED_MS)),
    );
    for holder in ["", "ci-8"] {
        let answer = apply_through_the_abi_as(&leased, &list, holder, WHILE_HELD_MS);
        assert_eq!(
            answer.exit,
            ExitClass::Refused as i32,
            "{holder:?}: {answer:?}"
        );
    }
    let answer = apply_through_the_abi_as(&leased, &list, "ci-7", WHILE_HELD_MS);
    assert_eq!(answer.exit, ExitClass::Done as i32, "{answer:?}");
}
