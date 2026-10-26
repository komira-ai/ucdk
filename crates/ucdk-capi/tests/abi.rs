// SPDX-License-Identifier: Apache-2.0
#![allow(unsafe_code)]
//! The exported functions, called in-process the way a C caller calls them.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

use common::{
    HookedLedger, Scratch, VerbFn, call, error_of, file_config, golden, memory_config,
    null_session, open, open_bytes, open_memory, take,
};
use prost::Message as _;
use ucdk_api::v1::{
    ErrorCode, ExitClass, LedgerShowRequest, LedgerShowResponse, Outcome, RecordOrigin,
    RecordState, SessionConfig, outcome,
};
use ucdk_capi::{
    ABI, ABI_MAJOR, ABI_MINOR, BUSY, Buf, DECODE, INVALID_ARGUMENT, MAX_REQUEST_LEN, OK, PANIC,
    Session, TOO_LARGE, into_handle, ucdk_abi_version, ucdk_buf_free, ucdk_ledger_show,
    ucdk_session_close, ucdk_session_open,
};
use ucdk_core::{Locator, RecordPath, Tag};
use ucdk_ledger::{Change, Ledger as _, MemoryLedger, Origin, Record, State};

const VERBS: [(&str, VerbFn); 1] = [("ledger_show", ucdk_ledger_show)];

fn close(session: *mut ucdk_capi::SessionHandle) {
    ucdk_session_close(session);
}

/// The answer of `ucdk_ledger_show` on `session`, decoded.
fn show(session: *mut ucdk_capi::SessionHandle) -> Outcome {
    let reply = call(
        ucdk_ledger_show,
        session,
        &LedgerShowRequest {}.encode_to_vec(),
    );
    assert_eq!(reply.status, OK);
    Outcome::decode(reply.bytes.as_slice()).expect("an outcome")
}

fn shown(answer: Outcome) -> LedgerShowResponse {
    assert_eq!(answer.exit, ExitClass::Done as i32, "{answer:?}");
    match answer.result {
        Some(outcome::Result::LedgerShow(response)) => response,
        other => panic!("not a ledger show: {other:?}"),
    }
}

/// The message of an answer that is the ledger error, with its code and class checked.
fn ledger_error(answer: Outcome) -> String {
    assert_eq!(answer.exit, ExitClass::FailedNoEffect as i32, "{answer:?}");
    match answer.result {
        Some(outcome::Result::Error(error)) => {
            assert_eq!(error.code, ErrorCode::Ledger as i32);
            error.message
        }
        other => panic!("not an error outcome: {other:?}"),
    }
}

/// Proves: the version is `(major << 16) | minor` of the constants the header repeats. Catches
/// an SDK-visible version that does not match the library's.
#[test]
fn the_version_packs_major_and_minor() {
    assert_eq!(ucdk_abi_version(), (ABI_MAJOR << 16) | ABI_MINOR);
}

/// Proves: a config that is not a SessionConfig is DECODE, leaves `*out_session` null and puts
/// an Error message in `*out_error`. Catches a garbage config that opens a session, or a failure
/// that leaves the output unset.
#[test]
fn a_garbage_config_is_refused_with_decode() {
    let opened = open_bytes(&[0xff, 0xff, 0xff]);
    assert_eq!(opened.status, DECODE);
    assert!(opened.session.is_null());
    assert_eq!(
        error_of(&opened.error).code,
        ErrorCode::InvalidRequest as i32
    );
    assert!(error_of(&opened.error).message.contains("SessionConfig"));
}

/// Proves: a ledger kind this library lacks is ABI, and a file kind with no location is
/// INVALID_ARGUMENT, each with a null session and a message. Catches a config that is
/// half-honoured, such as an ignored ledger kind.
#[test]
fn a_session_with_an_unusable_config_is_refused_at_open() {
    let with = |edit: fn(&mut SessionConfig)| {
        let mut config = memory_config();
        edit(&mut config);
        open(&config)
    };
    let cases = [
        (
            "unknown kind",
            ABI,
            with(|c| c.ledger_kind = "tape".to_owned()),
        ),
        (
            "file kind with no location",
            INVALID_ARGUMENT,
            with(|c| c.ledger_kind = "file".to_owned()),
        ),
    ];
    for (what, want, opened) in cases {
        assert_eq!(opened.status, want, "{what}");
        assert!(opened.session.is_null(), "{what}");
        assert!(!error_of(&opened.error).message.is_empty(), "{what}");
    }
}

/// Proves: the supported configs open: the empty config and the memory kind (a memory ledger),
/// and the file kind, whatever the file is: a file that is missing, a directory that does not
/// exist and a file that is not a ledger all open a session, because a file ledger is opened by
/// the call that shows it, not by the open. Catches a session that opens (and so locks or
/// creates) the file at open time.
#[test]
fn the_supported_configs_open() {
    for config in [SessionConfig::default(), memory_config()] {
        let opened = open(&config);
        assert_eq!(opened.status, OK, "{config:?}");
        close(opened.session);
    }

    let scratch = Scratch::new("supported-configs");
    let mut config = file_config(&scratch.file());
    let first = open(&config);
    assert_eq!(first.status, OK);
    let second = open(&config);
    assert_eq!(second.status, OK, "a ledger open in a first session opens");
    close(second.session);
    close(first.session);
    assert!(
        scratch.listing().is_empty(),
        "opening a session made no file, not even a lock file"
    );

    std::fs::write(scratch.file(), "not a ledger\n").expect("write");
    let corrupt = open(&config);
    assert_eq!(
        corrupt.status, OK,
        "a file that is not a ledger still opens"
    );
    assert!(!corrupt.session.is_null());
    close(corrupt.session);

    config.ledger_location = scratch
        .dir()
        .join("missing/ledger")
        .to_string_lossy()
        .into_owned();
    let missing = open(&config);
    assert_eq!(missing.status, OK);
    close(missing.session);
}

/// Proves: opening with no `out_session` is INVALID_ARGUMENT and still reports in `out_error`;
/// a null `out_error` is allowed; a null config with a length is INVALID_ARGUMENT and with no
/// length is the empty config, which is a memory ledger and opens. Catches a null dereference.
#[test]
fn open_checks_its_pointers() {
    let mut error = common::junk();
    // SAFETY: `out_error` is writable; the null `out_session` is what is under test.
    let status =
        unsafe { ucdk_session_open(std::ptr::null(), 0, std::ptr::null_mut(), &raw mut error) };
    assert_eq!(status, INVALID_ARGUMENT);
    assert!(error_of(&take(error)).message.contains("out_session"));

    let mut session = std::ptr::NonNull::dangling().as_ptr();
    // SAFETY: `out_session` is writable; a null `out_error` is allowed.
    let status =
        unsafe { ucdk_session_open(std::ptr::null(), 5, &raw mut session, std::ptr::null_mut()) };
    assert_eq!((status, session.is_null()), (INVALID_ARGUMENT, true));

    // SAFETY: a zero length reads nothing; both outputs are writable.
    let status =
        unsafe { ucdk_session_open(std::ptr::null(), 0, &raw mut session, &raw mut error) };
    assert_eq!((status, session.is_null()), (OK, false));
    assert!(take(error).is_empty(), "no error on success");
    close(session);

    // SAFETY: both pointers are null, which is allowed for a zero-length config with no error out.
    let status =
        unsafe { ucdk_session_open(std::ptr::null(), 0, &raw mut session, std::ptr::null_mut()) };
    assert_eq!(status, OK);
    close(session);
}

/// Proves: a request longer than the limit is TOO_LARGE before any byte is read (the pointer is
/// never dereferenced), for the open and for the verb. Catches a missing size check.
#[test]
fn a_request_over_the_limit_is_too_large() {
    let session = open_memory();
    let far = std::ptr::NonNull::<u8>::dangling().as_ptr();
    for (name, verb) in VERBS {
        let mut out = common::junk();
        // SAFETY: the pointer is not dereferenced: the length is over the limit.
        let status = unsafe { verb(session, far, MAX_REQUEST_LEN + 1, &raw mut out) };
        assert_eq!(status, TOO_LARGE, "{name}");
        assert_eq!(
            error_of(&take(out)).code,
            ErrorCode::InvalidRequest as i32,
            "{name}"
        );
    }
    let mut out = null_session();
    let mut error = common::junk();
    // SAFETY: as above.
    let status =
        unsafe { ucdk_session_open(far, MAX_REQUEST_LEN + 1, &raw mut out, &raw mut error) };
    assert_eq!(status, TOO_LARGE);
    take(error);
    close(session);
}

/// Proves: for the verb, a null or never-opened session is INVALID_ARGUMENT, a null `out` is
/// INVALID_ARGUMENT, a null request with a length is INVALID_ARGUMENT, and garbage bytes are
/// DECODE; each failure leaves an Error message in `out`. Catches a verb that dereferences what
/// it was not given or accepts bytes that are not its message.
#[test]
fn every_verb_checks_its_arguments() {
    let session = open_memory();
    // A struct on this stack that was never a session.
    let mut stranger = 0_u64;
    let stranger_handle = (&raw mut stranger).cast::<ucdk_capi::SessionHandle>();
    for (name, verb) in VERBS {
        for bad in [null_session(), stranger_handle] {
            let reply = call(verb, bad, &[]);
            assert_eq!(
                reply.status, INVALID_ARGUMENT,
                "{name}: not an open session"
            );
            assert!(
                error_of(&reply.bytes).message.contains("not open"),
                "{name}"
            );
        }
        // SAFETY: the null `out` is what is under test, and nothing else is read.
        let status = unsafe { verb(session, std::ptr::null(), 0, std::ptr::null_mut()) };
        assert_eq!(status, INVALID_ARGUMENT, "{name}: null out");

        let mut out = common::junk();
        // SAFETY: the null request with a length is under test; `out` is writable.
        let status = unsafe { verb(session, std::ptr::null(), 3, &raw mut out) };
        assert_eq!(status, INVALID_ARGUMENT, "{name}: null request");
        take(out);

        let reply = call(verb, session, &[0xff, 0xff, 0xff]);
        assert_eq!(reply.status, DECODE, "{name}: garbage");
        assert_eq!(
            error_of(&reply.bytes).code,
            ErrorCode::InvalidRequest as i32,
            "{name}"
        );
    }
    close(session);
}

/// Proves: a null request with a zero length is the empty message, and the empty message is a
/// whole request: a ledger show of an empty memory ledger answers OK with a document of version
/// 0, no lease and no records. Catches an empty request refused as a call error.
#[test]
fn a_null_request_with_no_length_is_the_empty_message() {
    let session = open_memory();
    let mut out = common::junk();
    // SAFETY: a zero length reads nothing; `out` is writable.
    let status = unsafe { ucdk_ledger_show(session, std::ptr::null(), 0, &raw mut out) };
    assert_eq!(status, OK);
    let answer = Outcome::decode(take(out).as_slice()).expect("an outcome");
    let response = shown(answer);
    assert_eq!(response.token, "v0");
    assert_eq!(response.fence, 0);
    assert!(response.lease.is_none() && response.records.is_empty());
    close(session);
}

/// Proves: use after close is refused, a second close and a close of null or of an address that
/// was never a session do nothing, and an unrelated session keeps working throughout. Catches a
/// dangling handle that is used, and a double close that frees twice.
#[test]
fn a_closed_session_is_refused_and_closing_again_is_harmless() {
    let doomed = open_memory();
    let other = open_memory();
    close(doomed);
    close(doomed);
    close(null_session());
    let mut stranger = 0_u64;
    close((&raw mut stranger).cast());
    for (name, verb) in VERBS {
        let reply = call(verb, doomed, &[]);
        assert_eq!(reply.status, INVALID_ARGUMENT, "{name}");
    }
    assert_eq!(call(ucdk_ledger_show, other, &[]).status, OK);
    close(other);
}

/// Proves: freeing a zeroed buffer, and one with a pointer and no length, does nothing, and a
/// real response is freed by `ucdk_buf_free`. Catches a free of memory the library did not
/// allocate.
#[test]
fn freeing_an_empty_buffer_is_a_no_op() {
    // SAFETY: a zeroed buffer is the documented empty buffer.
    unsafe { ucdk_buf_free(Buf::EMPTY) };
    // SAFETY: a zero length frees nothing, whatever the pointer.
    unsafe {
        ucdk_buf_free(Buf {
            ptr: std::ptr::NonNull::dangling().as_ptr(),
            len: 0,
        });
    }
    let session = open_memory();
    let reply = call(ucdk_ledger_show, session, &[]);
    assert_eq!(reply.status, OK);
    assert!(!reply.bytes.is_empty());
    close(session);
}

/// Proves: a ledger show through the ABI of the records golden answers its token, its fence, no
/// lease, and every record with its kind, name, origin, state, physical id, dependencies and
/// outputs in the orders the document has. Catches a response that drops or reorders a field.
#[test]
fn ledger_show_prints_token_fence_lease_and_records_of_the_golden() {
    let session = open(&file_config(&golden("records.ucdk"))).session;
    let response = shown(show(session));
    assert_eq!((response.token.as_str(), response.fence), ("v5", 0));
    assert!(response.lease.is_none());
    let paths: Vec<&str> = response.records.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "app/db",
            "app/old",
            "app/pending",
            "app/run",
            "shared/found"
        ]
    );
    let run = &response.records[3];
    assert_eq!(
        (run.kind.as_str(), run.name.as_str()),
        ("service", "app-run")
    );
    assert_eq!(run.physical, "run-1");
    assert_eq!(run.deps, ["app/db"]);
    assert_eq!(run.outputs.len(), 1);
    assert_eq!(
        (run.outputs[0].key.as_str(), run.outputs[0].value.as_str()),
        ("URL", "run-url")
    );
    let found = &response.records[4];
    assert_eq!(found.origin, RecordOrigin::Adopted as i32);
    let states: Vec<i32> = response.records.iter().map(|r| r.state).collect();
    assert_eq!(
        states,
        [
            RecordState::Confirmed as i32,
            RecordState::Reaping as i32,
            RecordState::Intent as i32,
            RecordState::Confirmed as i32,
            RecordState::Confirmed as i32,
        ]
    );
    close(session);
}

/// Proves: a ledger show takes no lease and changes no file: the directory's names and bytes
/// are equal before and after, and the lease in the file is the one that was there. Opening the
/// session did not make a lock file either. Catches a show that opens the file for writing, makes
/// the lock file, removes a temporary, or takes the lease to read.
#[test]
fn ledger_show_takes_no_lease_and_changes_no_file() {
    let scratch = Scratch::new("show-changes-nothing");
    std::fs::copy(golden("leased.ucdk"), scratch.file()).expect("copy the golden");
    // A write that never finished: a writer's open would remove it.
    std::fs::write(scratch.dir().join("ledger.ucdk.tmp"), b"half a write").expect("write");
    let before = scratch.listing();
    let session = open(&file_config(&scratch.file())).session;
    for _ in 0..2 {
        let response = shown(show(session));
        assert_eq!(response.fence, 1);
        let lease = response.lease.expect("the lease of the file");
        assert_eq!(lease.holder, "ci-7");
        assert_eq!(lease.expires_ms, 4_600_000);
    }
    close(session);
    assert_eq!(
        scratch.listing(),
        before,
        "names and bytes are as they were"
    );
}

/// Proves: a file that is not a ledger is the ledger error of the call, not a failed open: the
/// session opens, the call is status OK with an `error` outcome (`LEDGER`, class 4), and the file
/// is left as it was. Catches a corrupt ledger reported as a status code.
#[test]
fn ledger_show_of_a_corrupt_file_is_the_ledger_error() {
    let scratch = Scratch::new("show-corrupt");
    std::fs::write(scratch.file(), "this is not a ledger\n").expect("write");
    let session = open(&file_config(&scratch.file())).session;
    let message = ledger_error(show(session));
    assert!(
        message.contains("corrupt") || message.contains("not a ucdk ledger"),
        "{message}"
    );
    assert_eq!(
        std::fs::read_to_string(scratch.file()).expect("read"),
        "this is not a ledger\n"
    );
    // A file that does not exist is the same error, and no file is made for it.
    close(session);
    let missing = Scratch::new("show-missing");
    let session = open(&file_config(&missing.file())).session;
    let message = ledger_error(show(session));
    assert!(message.contains("does not exist"), "{message}");
    assert!(missing.listing().is_empty());
    close(session);
}

/// Proves: a ledger file of a newer major is refused by name: the message says which version it
/// is and that it is newer than this build reads. Catches a newer file read as if it were ours.
#[test]
fn ledger_show_of_a_newer_major_is_refused_by_name() {
    let scratch = Scratch::new("show-newer");
    let body = "ucdk-ledger 9.0\nversion 1\n";
    let sum = body.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    std::fs::write(scratch.file(), format!("{body}end 0 {sum:016x}\n")).expect("write");
    let session = open(&file_config(&scratch.file())).session;
    let message = ledger_error(show(session));
    assert!(
        message.contains("version 9") && message.contains("newer"),
        "{message}"
    );
    close(session);
}

/// Proves: a ledger that holds no lease shows none, with the fence it left: the document has
/// records and no lease line, so the `lease` field is absent, not an empty message. Catches a
/// lease reported when none is recorded.
#[test]
fn ledger_show_of_an_unleased_ledger_shows_no_lease() {
    let ledger = MemoryLedger::new();
    let lease = ledger
        .acquire("ci-7", std::time::Duration::from_secs(60))
        .expect("acquire");
    ledger.release(lease.fence).expect("release");
    let session = into_handle(Session::with_ledger(Box::new(ledger)));
    let response = shown(show(session));
    assert!(response.lease.is_none());
    assert_eq!(response.fence, 1, "the highest fence ever granted stays");
    close(session);
}

/// Proves: every state and origin a record can have is mapped through the ABI to the enum of the
/// message, and an empty physical id and no dependencies or outputs show as empty. Catches a
/// state or an origin that is mapped to its neighbour.
#[test]
fn ledger_show_maps_every_record_state_and_origin() {
    let ledger = MemoryLedger::new();
    let states = [State::Intent, State::Confirmed, State::Reaping];
    let origins = [Origin::Created, Origin::Adopted];
    let mut wanted = BTreeSet::new();
    for (s, state) in states.into_iter().enumerate() {
        for (o, origin) in origins.into_iter().enumerate() {
            let path = RecordPath::new(&format!("a/s{s}o{o}")).expect("a path");
            let record = Record {
                origin,
                state,
                ..Record::intent(path, Tag::new("thing"), "n", BTreeSet::new())
            };
            let token = ledger.read().expect("read").token;
            ledger.write(&token, Change::Put(record)).expect("write");
            wanted.insert((format!("a/s{s}o{o}"), o as i32 + 1, s as i32 + 1));
        }
    }
    let session = into_handle(Session::with_ledger(Box::new(ledger)));
    let response = shown(show(session));
    let found: BTreeSet<(String, i32, i32)> = response
        .records
        .iter()
        .map(|r| (r.path.clone(), r.origin, r.state))
        .collect();
    assert_eq!(found, wanted);
    assert!(
        response
            .records
            .iter()
            .all(|r| r.physical.is_empty() && r.deps.is_empty())
    );
    // The numbers are the enums' own: created 1, adopted 2; intent 1, confirmed 2, reaping 3.
    assert_eq!(RecordOrigin::Adopted as i32, 2);
    assert_eq!(RecordState::Reaping as i32, 3);
    close(session);
}

/// Proves: a record with a physical id, dependencies and outputs set through a ledger the host
/// holds is shown with all of them. Catches a field that is read and not shown.
#[test]
fn a_ledger_the_host_holds_is_shown_as_it_is() {
    let ledger = MemoryLedger::new();
    let record = Record {
        physical: Some(Locator::new("id-1")),
        deps: [RecordPath::new("a/db").expect("a path")].into(),
        outputs: [("K".to_owned(), "v".to_owned())].into(),
        ..Record::intent(
            RecordPath::new("a/run").expect("a path"),
            Tag::new("thing"),
            "n",
            BTreeSet::new(),
        )
    };
    let token = ledger.read().expect("read").token;
    ledger.write(&token, Change::Put(record)).expect("write");
    let session = into_handle(Session::with_ledger(Box::new(ledger)).with_holder("ci-7"));
    let response = shown(show(session));
    let shown = &response.records[0];
    assert_eq!(shown.physical, "id-1");
    assert_eq!(shown.deps, ["a/db"]);
    assert_eq!(
        (
            shown.outputs[0].key.as_str(),
            shown.outputs[0].value.as_str()
        ),
        ("K", "v")
    );
    close(session);
}

/// Proves: sessions keep separate worlds: two sessions on different ledgers show their own.
/// Catches state shared between sessions.
#[test]
fn sessions_are_independent() {
    let with_record = MemoryLedger::new();
    let token = with_record.read().expect("read").token;
    let record = Record::intent(
        RecordPath::new("a/run").expect("a path"),
        Tag::new("thing"),
        "n",
        BTreeSet::new(),
    );
    with_record
        .write(&token, Change::Put(record))
        .expect("write");
    let first = into_handle(Session::with_ledger(Box::new(with_record)));
    let second = open_memory();
    assert_eq!(shown(show(first)).records.len(), 1);
    assert!(shown(show(second)).records.is_empty());
    close(first);
    close(second);
}

fn hooked_session(hook: impl FnMut() + Send + 'static) -> *mut ucdk_capi::SessionHandle {
    into_handle(Session::with_ledger(Box::new(HookedLedger::new(hook))))
}

/// Proves: a panic inside a call is contained (the test process lives), answered PANIC with the
/// panic's text for each kind of payload, and poisons the session: the next call on it is PANIC
/// again and says so, close still works, and another session is unaffected. Catches an unwind
/// into the caller, a session that keeps serving after a panic, and a poisoned session that
/// cannot be closed.
#[test]
fn a_panic_is_contained_and_poisons_the_session() {
    let panics: [(&str, fn()); 3] = [
        ("a literal", || panic!("boom literal")),
        ("a formatted text", || {
            std::panic::panic_any(String::from("boom formatted"));
        }),
        ("no text", || std::panic::panic_any(7_u32)),
    ];
    let healthy = open_memory();
    for (what, hook) in panics {
        let session = hooked_session(hook);
        let reply = call(ucdk_ledger_show, session, &[]);
        assert_eq!(reply.status, PANIC, "{what}");
        let message = error_of(&reply.bytes).message;
        assert!(message.contains("panic contained"), "{what}: {message}");
        if what == "a literal" {
            assert!(message.contains("boom literal"), "{message}");
        }
        if what == "a formatted text" {
            assert!(message.contains("boom formatted"), "{message}");
        }
        for (name, verb) in VERBS {
            let again = call(verb, session, &[]);
            assert_eq!(again.status, PANIC, "{what}: {name} after the panic");
            assert!(
                error_of(&again.bytes).message.contains("poisoned"),
                "{what}: {name}"
            );
        }
        assert_eq!(
            call(ucdk_ledger_show, healthy, &[]).status,
            OK,
            "{what}: another session"
        );
        close(session);
        assert_eq!(
            call(ucdk_ledger_show, session, &[]).status,
            INVALID_ARGUMENT,
            "{what}: after close"
        );
    }
    close(healthy);
}

/// Proves: a call made on a session while a call on it is running returns BUSY at once, does not
/// queue, and does not disturb the running call, which finishes with OK; afterwards the session
/// serves again. Catches a re-entrant call that deadlocks, runs concurrently or corrupts the
/// first.
#[test]
fn a_second_call_on_a_busy_session_is_refused() {
    let handle = Arc::new(AtomicUsize::new(0));
    let inner = Arc::new(AtomicI32::new(-1));
    let (h, i) = (Arc::clone(&handle), Arc::clone(&inner));
    let session = hooked_session(move || {
        let session = h.load(Ordering::SeqCst) as *mut ucdk_capi::SessionHandle;
        let reply = call(ucdk_ledger_show, session, &[]);
        i.store(reply.status, Ordering::SeqCst);
    });
    handle.store(session as usize, Ordering::SeqCst);

    let outer = call(ucdk_ledger_show, session, &[]);
    assert_eq!(outer.status, OK);
    assert_eq!(inner.load(Ordering::SeqCst), BUSY);
    let outcome = Outcome::decode(outer.bytes.as_slice()).expect("an outcome");
    assert!(matches!(
        outcome.result,
        Some(outcome::Result::LedgerShow(_))
    ));

    inner.store(-1, Ordering::SeqCst);
    let after = call(ucdk_ledger_show, session, &[]);
    assert_eq!(after.status, OK, "the session serves again");
    close(session);
}

/// Proves: a handle stays refused after it is closed even when a later session is opened, so a
/// stale handle can never reach the new session, whatever the allocator does with the address.
/// Catches a handle that is an address a new session can be given again.
#[test]
fn a_closed_handle_stays_refused_after_another_session_opens() {
    let first = open_memory();
    close(first);
    let second = open_memory();
    assert_ne!(first, second, "a handle is never handed out twice");
    assert_eq!(call(ucdk_ledger_show, first, &[]).status, INVALID_ARGUMENT);
    assert_eq!(call(ucdk_ledger_show, second, &[]).status, OK);
    close(second);
}

/// Proves: when a session is closed during a call, so the call holds the last reference and
/// drops the session itself, a destructor that panics is contained: the call answers PANIC and
/// the process lives. Catches a session dropped outside `catch_unwind`, where the panic would
/// unwind out of an `extern "C"` function and abort.
#[test]
fn a_panicking_destructor_after_a_concurrent_close_is_contained() {
    let handle = Arc::new(AtomicUsize::new(0));
    let h = Arc::clone(&handle);
    let mut ledger = HookedLedger::new(move || {
        close(h.load(Ordering::SeqCst) as *mut ucdk_capi::SessionHandle);
    });
    ledger.panic_on_drop = true;
    let session = into_handle(Session::with_ledger(Box::new(ledger)));
    handle.store(session as usize, Ordering::SeqCst);

    let reply = call(ucdk_ledger_show, session, &[]);
    assert_eq!(reply.status, PANIC);
    assert!(error_of(&reply.bytes).message.contains("destructor"));
    assert_eq!(
        call(ucdk_ledger_show, session, &[]).status,
        INVALID_ARGUMENT
    );
}
