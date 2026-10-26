// SPDX-License-Identifier: Apache-2.0
#![allow(unsafe_code)]
//! The exported functions, called in-process the way a C caller calls them.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

use common::{
    Hooked, Opened, Reply, VerbFn, call, error_of, fake_config, list_of, null_session, open,
    open_bytes, open_fake, service_of, take,
};
use prost::Message as _;
use std::time::Duration;
use ucdk_api::catalog_response;
use ucdk_api::v1::{
    self, ApplyRequest, CatalogResponse, DestroyRequest, ErrorCode, ExitClass, LeaksRequest, Mode,
    Outcome, PlanRequest, SessionConfig, StatusRequest, outcome,
};
use ucdk_capi::{
    ABI, ABI_MAJOR, ABI_MINOR, BUSY, Buf, DECODE, INTERNAL, INVALID_ARGUMENT, MAX_REQUEST_LEN, OK,
    PANIC, Session, TOO_LARGE, into_handle, ucdk_abi_version, ucdk_apply, ucdk_buf_free,
    ucdk_destroy, ucdk_leaks, ucdk_load_catalog, ucdk_plan, ucdk_session_close, ucdk_session_open,
    ucdk_status,
};
use ucdk_engine::{Engine, Request};

use ucdk_ledger::{FileLedger, Ledger as _, ManualClock, MemoryLedger};
use ucdk_plan::lower;
use ucdk_proto::v1::ResourceList;
use ucdk_provider::kit;
use ucdk_provider_fake::Fake;

const VERBS: [(&str, VerbFn); 6] = [
    ("load_catalog", ucdk_load_catalog),
    ("plan", ucdk_plan),
    ("apply", ucdk_apply),
    ("destroy", ucdk_destroy),
    ("status", ucdk_status),
    ("leaks", ucdk_leaks),
];

fn close(session: *mut ucdk_capi::SessionHandle) {
    ucdk_session_close(session);
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

/// Proves: every way a config can ask for something this library lacks is ABI, and every other
/// bad config is INVALID_ARGUMENT or INTERNAL, each with a null session. Catches a config that is
/// half-honoured, such as an ignored provider option.
#[test]
fn a_config_that_asks_for_what_is_not_here_is_refused() {
    let with = |edit: fn(&mut SessionConfig)| {
        let mut config = fake_config();
        edit(&mut config);
        open(&config)
    };
    let cases: [(&str, i32, Opened); 8] = [
        ("empty provider", ABI, open(&SessionConfig::default())),
        (
            "unknown provider",
            ABI,
            with(|c| c.provider = "elsewhere".to_owned()),
        ),
        (
            "provider option",
            ABI,
            with(|c| {
                c.provider_options
                    .insert("region".to_owned(), "r".to_owned());
            }),
        ),
        ("credential source", ABI, with(|c| c.credentials = 99)),
        (
            "ledger kind",
            ABI,
            with(|c| c.ledger_kind = "tape".to_owned()),
        ),
        (
            "file ledger without location",
            INVALID_ARGUMENT,
            with(|c| c.ledger_kind = "file".to_owned()),
        ),
        (
            "cell",
            INVALID_ARGUMENT,
            with(|c| c.cell = "one/two".to_owned()),
        ),
        (
            "cell parts",
            INVALID_ARGUMENT,
            with(|c| c.cell = "A/b/c".to_owned()),
        ),
    ];
    for (what, want, opened) in cases {
        assert_eq!(opened.status, want, "{what}");
        assert!(opened.session.is_null(), "{what}");
        assert!(!error_of(&opened.error).message.is_empty(), "{what}");
    }
}

/// Proves: the supported providers and ledgers open; a config with no cell gets the local one;
/// a file ledger that cannot be opened (a directory that does not exist, or a file that is not
/// a ledger) is INTERNAL, while a second session on a ledger a first has open is fine: the lease
/// arbitrates between their runs. Catches a ledger failure reported as success, and a session
/// that keeps others out of the file.
#[test]
fn the_supported_configs_open_and_a_ledger_that_cannot_open_is_internal() {
    for provider in ["fake", "fake-limited", "fake-leaky"] {
        let mut config = fake_config();
        config.provider = provider.to_owned();
        config.ledger_kind = "memory".to_owned();
        let opened = open(&config);
        assert_eq!(opened.status, OK, "{provider}");
        close(opened.session);
    }
    let local = open(&SessionConfig {
        provider: "fake".to_owned(),
        ..SessionConfig::default()
    });
    assert_eq!(local.status, OK);
    close(local.session);

    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("abi-file-ledger");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let mut config = fake_config();
    config.ledger_kind = "file".to_owned();
    config.ledger_location = dir.join("ledger").to_string_lossy().into_owned();
    let first = open(&config);
    assert_eq!(first.status, OK);
    let second = open(&config);
    assert_eq!(second.status, OK, "a ledger open in a first session opens");
    close(second.session);
    close(first.session);

    std::fs::write(dir.join("ledger"), "not a ledger\n").expect("write");
    let corrupt = open(&config);
    assert_eq!(corrupt.status, INTERNAL, "the file is not a ledger");
    assert!(corrupt.session.is_null());

    config.ledger_location = dir.join("missing/ledger").to_string_lossy().into_owned();
    assert_eq!(open(&config).status, INTERNAL);
}

/// Proves: opening with no `out_session` is INVALID_ARGUMENT and still reports in `out_error`;
/// a null `out_error` is allowed; a null config with a length is INVALID_ARGUMENT and with no
/// length is the empty config (refused, not a crash). Catches a null dereference.
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
    assert_eq!((status, session.is_null()), (ABI, true));
    take(error);

    // SAFETY: both pointers are null, which is allowed for a zero-length config with no error out.
    let status =
        unsafe { ucdk_session_open(std::ptr::null(), 0, &raw mut session, std::ptr::null_mut()) };
    assert_eq!(status, ABI);
}

/// Proves: a request longer than the limit is TOO_LARGE before any byte is read (the pointer is
/// never dereferenced), for the open and for every verb. Catches a missing size check.
#[test]
fn a_request_over_the_limit_is_too_large() {
    let session = open_fake();
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

/// Proves: for every verb, a null or never-opened session is INVALID_ARGUMENT, a null `out` is
/// INVALID_ARGUMENT, a null request with a length is INVALID_ARGUMENT, and garbage bytes are
/// DECODE; each failure leaves an Error message in `out`. Catches a verb that dereferences what
/// it was not given or accepts bytes that are not its message.
#[test]
fn every_verb_checks_its_arguments() {
    let session = open_fake();
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

/// Proves: a null request with a zero length is the empty message: an empty list loads with no
/// nodes, and an empty plan request is answered as data (a mode that is neither apply nor
/// destroy, exit class usage) with status OK. Catches an empty request refused as a call error,
/// or an invalid request reported through the status code.
#[test]
fn a_null_request_with_no_length_is_the_empty_message() {
    let session = open_fake();
    let mut out = common::junk();
    // SAFETY: a zero length reads nothing; `out` is writable.
    let status = unsafe { ucdk_load_catalog(session, std::ptr::null(), 0, &raw mut out) };
    assert_eq!(status, OK);
    let catalog = CatalogResponse::decode(take(out).as_slice()).expect("a catalog response");
    assert!(catalog.nodes.is_empty() && catalog.findings.is_empty());

    let reply = call(ucdk_plan, session, &[]);
    assert_eq!(reply.status, OK);
    let answer = Outcome::decode(reply.bytes.as_slice()).expect("an outcome");
    assert_eq!(answer.exit, ExitClass::Usage as i32);
    assert!(matches!(answer.result, Some(outcome::Result::Invalid(_))));
    close(session);
}

/// Proves: use after close is refused, a second close and a close of null or of an address that
/// was never a session do nothing, and an unrelated session keeps working throughout. Catches a
/// dangling handle that is used, and a double close that frees twice.
#[test]
fn a_closed_session_is_refused_and_closing_again_is_harmless() {
    let doomed = open_fake();
    let other = open_fake();
    close(doomed);
    close(doomed);
    close(null_session());
    let mut stranger = 0_u64;
    close((&raw mut stranger).cast());
    for (name, verb) in VERBS {
        let reply = call(verb, doomed, &[]);
        assert_eq!(reply.status, INVALID_ARGUMENT, "{name}");
    }
    assert_eq!(call(ucdk_status, other, &[]).status, OK);
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
    let session = open_fake();
    let reply = call(ucdk_status, session, &[]);
    assert_eq!(reply.status, OK);
    assert!(!reply.bytes.is_empty());
    close(session);
}

/// A session through the ABI and, beside it, the engine driven directly over a world of its own
/// with the same provider, ledger and cell. Every step asserts that the two answer with the same
/// bytes.
struct Both {
    session: *mut ucdk_capi::SessionHandle,
    fake: Fake,
    ledger: MemoryLedger,
}

impl Both {
    fn new() -> Self {
        Self {
            session: open_fake(),
            fake: Fake::standard(),
            ledger: MemoryLedger::new(),
        }
    }

    fn engine(&mut self, request: Request) -> ucdk_engine::Outcome {
        Engine::new(&mut self.fake, &self.ledger, kit::cell()).handle(request)
    }

    fn plan(&mut self, list: &ResourceList, mode: Mode) -> Outcome {
        let request = PlanRequest {
            resources: Some(list.clone()),
            mode: mode as i32,
        };
        let wire = call(ucdk_plan, self.session, &request.encode_to_vec());
        assert_eq!(wire.status, OK);
        let direct = match request.to_request() {
            Ok(request) => Outcome::from(&self.engine(request)),
            Err(refused) => refused.into_outcome(),
        };
        assert_eq!(
            wire.bytes,
            direct.encode_to_vec(),
            "plan: wire and engine agree"
        );
        direct
    }

    fn status(&mut self, list: &ResourceList) -> Outcome {
        let request = StatusRequest {
            resources: Some(list.clone()),
        };
        let wire = call(ucdk_status, self.session, &request.encode_to_vec());
        assert_eq!(wire.status, OK);
        let direct = Outcome::from(&self.engine(request.to_request().expect("a request")));
        assert_eq!(
            wire.bytes,
            direct.encode_to_vec(),
            "status: wire and engine agree"
        );
        direct
    }

    fn leaks(&mut self) -> Outcome {
        let request = LeaksRequest {};
        let wire = call(ucdk_leaks, self.session, &request.encode_to_vec());
        assert_eq!(wire.status, OK);
        let direct = Outcome::from(&self.engine(request.to_request()));
        assert_eq!(
            wire.bytes,
            direct.encode_to_vec(),
            "leaks: wire and engine agree"
        );
        direct
    }

    /// An apply or a destroy: the engine plans the world as it is, then runs with the pinned hash.
    fn run(&mut self, destroy: bool, list: &ResourceList, hash: &str) -> Outcome {
        let resources = Some(list.clone());
        let plan_hash = hash.to_owned();
        let (verb, mode, approval): (VerbFn, Mode, _) = if destroy {
            let request = DestroyRequest {
                resources: resources.clone(),
                plan_hash,
            };
            let wire = call(ucdk_destroy, self.session, &request.encode_to_vec());
            (ucdk_destroy, Mode::Destroy, (wire, request.approved()))
        } else {
            let request = ApplyRequest {
                resources: resources.clone(),
                plan_hash,
                rollback: false,
            };
            let wire = call(ucdk_apply, self.session, &request.encode_to_vec());
            (ucdk_apply, Mode::Apply, (wire, request.approved()))
        };
        let _ = verb;
        let (wire, approval): (Reply, _) = approval;
        assert_eq!(wire.status, OK);
        let direct = match approval {
            Err(refused) => refused.into_outcome(),
            Ok(approved) => {
                let planned = PlanRequest {
                    resources,
                    mode: mode as i32,
                }
                .to_request()
                .expect("a request");
                match self.engine(planned) {
                    ucdk_engine::Outcome::Planned(plan) => {
                        let request = if destroy {
                            approved.destroy(plan)
                        } else {
                            approved.apply(plan)
                        };
                        Outcome::from(&self.engine(request))
                    }
                    other => Outcome::from(&other),
                }
            }
        };
        assert_eq!(
            wire.bytes,
            direct.encode_to_vec(),
            "run: wire and engine agree"
        );
        direct
    }
}

fn planned_hash(answer: &Outcome) -> String {
    match &answer.result {
        Some(outcome::Result::Planned(plan)) => plan.hash.clone(),
        other => panic!("not a plan: {other:?}"),
    }
}

fn standings(answer: &Outcome) -> Vec<i32> {
    match &answer.result {
        Some(outcome::Result::Status(status)) => {
            status.objects.iter().map(|o| o.standing).collect()
        }
        other => panic!("not a status: {other:?}"),
    }
}

/// Proves: plan, apply, status and destroy through the ABI give byte-for-byte the answers the
/// engine gives when driven directly: a plan with its hash, an apply under that hash that creates
/// every node, a status that finds them all in agreement, a second plan that changes nothing, a
/// destroy under its own hash, and a status that finds nothing left. Catches any behaviour the C
/// layer adds, drops or reorders.
#[test]
fn a_run_through_the_abi_is_the_run_of_the_engine() {
    let list = list_of(vec![service_of("web", 80), service_of("db", 5432)]);
    let mut both = Both::new();

    let planned = both.plan(&list, Mode::Apply);
    let hash = planned_hash(&planned);
    assert_eq!(planned.exit, ExitClass::Done as i32);

    let done = both.run(false, &list, &hash);
    let Some(outcome::Result::Done(report)) = &done.result else {
        panic!("not done: {done:?}");
    };
    assert_eq!(report.created.len(), 2, "{report:?}");

    let status = both.status(&list);
    assert_eq!(standings(&status), [v1::Standing::Agrees as i32; 2]);

    let leaks = both.leaks();
    assert_eq!(leaks.exit, ExitClass::Done as i32);
    assert!(
        matches!(&leaks.result, Some(outcome::Result::Leaks(l)) if l.leaks.is_empty()),
        "an applied list leaks nothing: {leaks:?}"
    );

    let again = both.plan(&list, Mode::Apply);
    let Some(outcome::Result::Planned(plan)) = &again.result else {
        panic!("not planned");
    };
    assert!(
        plan.steps
            .iter()
            .all(|s| s.action == v1::StepAction::Noop as i32),
        "{plan:?}"
    );

    let destroy = both.plan(&list, Mode::Destroy);
    let gone = both.run(true, &list, &planned_hash(&destroy));
    let Some(outcome::Result::Done(report)) = &gone.result else {
        panic!("not done: {gone:?}");
    };
    assert_eq!(report.deleted.len(), 2, "{report:?}");
    assert!(
        standings(&both.status(&list))
            .iter()
            .all(|s| *s == v1::Standing::NotApplied as i32)
    );
    close(both.session);
}

/// Proves: a plan that is not the one pinned is data, not a call error: an apply under a hash
/// that is not the plan's answers OK with a refusal naming both hashes and changes nothing; a
/// hash that is not 64 hex digits and a list that is refused for findings are answered the same
/// way; each equals what the engine says directly. Catches an outcome reported as a status code,
/// and an apply that runs under the wrong hash.
#[test]
fn refusals_are_answers_and_change_nothing() {
    let list = list_of(vec![service_of("web", 80)]);
    let mut both = Both::new();
    let wrong = "a".repeat(64);
    let refused = both.run(false, &list, &wrong);
    assert_eq!(refused.exit, ExitClass::Refused as i32);
    let Some(outcome::Result::Refused(why)) = &refused.result else {
        panic!("not refused");
    };
    assert!(
        matches!(why.reason, Some(v1::refusal::Reason::PlanHashMismatch(_))),
        "{why:?}"
    );
    assert!(
        standings(&both.status(&list))
            .iter()
            .all(|s| *s == v1::Standing::NotApplied as i32)
    );

    let malformed = both.run(false, &list, "not a hash");
    assert_eq!(malformed.exit, ExitClass::Usage as i32);
    let destroyed = both.run(true, &list, "not a hash");
    assert_eq!(destroyed.exit, ExitClass::Usage as i32);

    let bad = list_of(vec![service_of("web", 80), service_of("web", 81)]);
    let findings = both.run(false, &bad, &wrong);
    assert_eq!(findings.exit, ExitClass::Refused as i32);
    let findings = both.run(true, &bad, &wrong);
    assert_eq!(findings.exit, ExitClass::Refused as i32);
    close(both.session);
}

/// Proves: loading a catalog answers with the lowered nodes, or with the findings that refuse
/// the list, and equals what `ucdk-api` makes of `lower`. Catches a catalog that differs from the
/// planner's.
#[test]
fn load_catalog_answers_with_the_nodes_or_the_findings() {
    let session = open_fake();
    for list in [
        list_of(vec![service_of("web", 80)]),
        list_of(vec![service_of("web", 80), service_of("web", 81)]),
    ] {
        let reply = call(ucdk_load_catalog, session, &list.encode_to_vec());
        assert_eq!(reply.status, OK);
        assert_eq!(reply.bytes, catalog_response(lower(&list)).encode_to_vec());
    }
    close(session);
}

/// Proves: a session with a status in flight on another session is unaffected, and sessions
/// keep separate worlds: what one applies the other does not see. Catches shared state between
/// sessions.
#[test]
fn sessions_are_independent() {
    let list = list_of(vec![service_of("web", 80)]);
    let mut first = Both::new();
    let second = open_fake();
    let hash = planned_hash(&first.plan(&list, Mode::Apply));
    first.run(false, &list, &hash);

    let request = StatusRequest {
        resources: Some(list),
    };
    let reply = call(ucdk_status, second, &request.encode_to_vec());
    let answer = Outcome::decode(reply.bytes.as_slice()).expect("an outcome");
    assert!(
        standings(&answer)
            .iter()
            .all(|s| *s == v1::Standing::NotApplied as i32)
    );
    close(first.session);
    close(second);
}

fn hooked_session(hook: impl FnMut() + Send + 'static) -> *mut ucdk_capi::SessionHandle {
    into_handle(Session::with_parts(
        Box::new(Hooked::new(hook)),
        Box::new(MemoryLedger::new()),
        kit::cell(),
    ))
}

fn plan_request(port: u32) -> Vec<u8> {
    PlanRequest {
        resources: Some(list_of(vec![service_of("web", port)])),
        mode: Mode::Apply as i32,
    }
    .encode_to_vec()
}

/// Proves: a panic inside a call is contained (the test process lives), answered PANIC with the
/// panic's text for each kind of payload, and poisons the session: the next call on it, of any
/// verb, is PANIC again and says so, close still works, and another session is unaffected.
/// Catches an unwind into the caller, a session that keeps serving after a panic, and a
/// poisoned session that cannot be closed.
#[test]
fn a_panic_is_contained_and_poisons_the_session() {
    let panics: [(&str, fn()); 3] = [
        ("a literal", || panic!("boom literal")),
        ("a formatted text", || {
            std::panic::panic_any(String::from("boom formatted"));
        }),
        ("no text", || std::panic::panic_any(7_u32)),
    ];
    let healthy = open_fake();
    for (what, hook) in panics {
        let session = hooked_session(hook);
        let reply = call(ucdk_plan, session, &plan_request(80));
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
            call(ucdk_status, healthy, &[]).status,
            OK,
            "{what}: another session"
        );
        close(session);
        assert_eq!(
            call(ucdk_status, session, &[]).status,
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
        let reply = call(ucdk_status, session, &[]);
        i.store(reply.status, Ordering::SeqCst);
    });
    handle.store(session as usize, Ordering::SeqCst);

    let outer = call(ucdk_plan, session, &plan_request(80));
    assert_eq!(outer.status, OK);
    assert_eq!(inner.load(Ordering::SeqCst), BUSY);
    let outcome = Outcome::decode(outer.bytes.as_slice()).expect("an outcome");
    assert!(matches!(outcome.result, Some(outcome::Result::Planned(_))));

    inner.store(-1, Ordering::SeqCst);
    let after = call(ucdk_status, session, &[]);
    assert_eq!(after.status, OK, "the session serves again");
    close(session);
}

/// Proves: when the plan the engine makes of the world as it is cannot be run (a foreign object
/// stands where a wanted one would go), an apply answers OK with that refusal as data, names the
/// conflict, and creates nothing; it is not a call error and the pinned hash is not consulted.
/// Catches an apply that runs over an object that is not ours.
#[test]
fn an_apply_over_a_foreign_object_is_refused_as_data() {
    let list = list_of(vec![service_of("web", 80)]);
    let node = lower(&list).expect("lowers").nodes()[0].node.clone();
    let mut provider = Hooked::new(|| {});
    provider.fake.add_foreign(node.kind(), node.name(), None);
    let session = into_handle(Session::with_parts(
        Box::new(provider),
        Box::new(MemoryLedger::new()),
        kit::cell(),
    ));
    let request = ApplyRequest {
        resources: Some(list),
        plan_hash: "a".repeat(64),
        rollback: false,
    };
    let reply = call(ucdk_apply, session, &request.encode_to_vec());
    assert_eq!(reply.status, OK);
    let answer = Outcome::decode(reply.bytes.as_slice()).expect("an outcome");
    assert_eq!(answer.exit, ExitClass::Refused as i32);
    let Some(outcome::Result::Refused(why)) = answer.result else {
        panic!("not refused");
    };
    assert!(
        matches!(why.reason, Some(v1::refusal::Reason::Conflicts(_))),
        "{why:?}"
    );
    close(session);
}

fn job_of(id: &str) -> ucdk_proto::v1::Resource {
    let mut resource = service_of(id, 80);
    let Some(ucdk_proto::v1::resource::Body::Service(service)) = resource.body.take() else {
        panic!("a service");
    };
    resource.body = Some(ucdk_proto::v1::resource::Body::Job(ucdk_proto::v1::Job {
        image: service.image,
        ..ucdk_proto::v1::Job::default()
    }));
    resource
}

/// Proves: a list holding a kind the provider does not host is refused whole through the ABI,
/// for plan and for apply under a hash a hosting provider gave it, and nothing is created, not
/// even the hosted resource that sorts first. Catches an apply that creates the hosted objects
/// and stops at the unhosted one.
#[test]
fn a_list_with_an_unhosted_kind_is_refused_before_any_write() {
    // `a-svc` is hosted and sorts before `z-job`, which `fake-limited` does not host.
    let list = list_of(vec![service_of("a-svc", 80), job_of("z-job")]);
    let mut hosting = Both::new();
    let hash = planned_hash(&hosting.plan(&list, Mode::Apply));
    close(hosting.session);

    let mut config = fake_config();
    config.provider = "fake-limited".to_owned();
    let opened = open(&config);
    assert_eq!(opened.status, OK);
    let session = opened.session;
    let refused = |reply: Reply| {
        assert_eq!(reply.status, OK);
        let answer = Outcome::decode(reply.bytes.as_slice()).expect("an outcome");
        assert_eq!(answer.exit, ExitClass::Refused as i32, "{answer:?}");
        match answer.result {
            Some(outcome::Result::Refused(why)) => match why.reason {
                Some(v1::refusal::Reason::UnhostedKinds(kinds)) => kinds,
                other => panic!("not an unhosted refusal: {other:?}"),
            },
            other => panic!("not refused: {other:?}"),
        }
    };
    let plan = PlanRequest {
        resources: Some(list.clone()),
        mode: Mode::Apply as i32,
    };
    let kinds = refused(call(ucdk_plan, session, &plan.encode_to_vec()));
    assert_eq!(kinds.kinds.len(), 1);
    assert_eq!(kinds.kinds[0].path, "z-job/run");
    assert_eq!(kinds.kinds[0].kind, "job");

    let apply = ApplyRequest {
        resources: Some(list),
        plan_hash: hash,
        rollback: false,
    };
    refused(call(ucdk_apply, session, &apply.encode_to_vec()));

    let only_hosted = StatusRequest {
        resources: Some(list_of(vec![service_of("a-svc", 80)])),
    };
    let reply = call(ucdk_status, session, &only_hosted.encode_to_vec());
    let answer = Outcome::decode(reply.bytes.as_slice()).expect("an outcome");
    assert_eq!(
        standings(&answer),
        [v1::Standing::NotApplied as i32],
        "nothing was created"
    );
    close(session);
}

/// A file ledger in a fresh directory of its own that `holder` leased for `ttl` before the
/// session opens it, on `clock`.
fn file_ledger_leased_by(
    name: &str,
    holder: &str,
    ttl: Duration,
    clock: Arc<dyn ucdk_ledger::Clock>,
) -> String {
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
fn apply_through_the_abi(location: &str, list: &ResourceList) -> Outcome {
    let mut config = fake_config();
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
        Duration::from_secs(3600),
        Arc::new(ucdk_ledger::SystemClock) as Arc<dyn ucdk_ledger::Clock>,
    );
    let answer = apply_through_the_abi(&live, &list);
    assert_eq!(answer.exit, ExitClass::Refused as i32, "{answer:?}");
    let Some(outcome::Result::Refused(why)) = answer.result else {
        panic!("not refused: {answer:?}");
    };
    let Some(v1::refusal::Reason::LeaseHeld(held)) = why.reason else {
        panic!("not a lease refusal: {why:?}");
    };
    assert_eq!(held.holder, "ci-7");
    assert!(held.remaining_ms > 0);

    // A holder that died long ago: its lease was granted in 1970 by a clock at 1 s.
    let dead = file_ledger_leased_by(
        "abi-lapsed",
        "ci-7",
        Duration::from_secs(60),
        Arc::new(ManualClock::at(1_000)),
    );
    let answer = apply_through_the_abi(&dead, &list);
    assert_eq!(answer.exit, ExitClass::Done as i32, "{answer:?}");
}

/// Proves: a handle stays refused after it is closed even when a later session is opened, so a
/// stale handle can never reach the new session, whatever the allocator does with the address.
/// Catches a handle that is an address a new session can be given again.
#[test]
fn a_closed_handle_stays_refused_after_another_session_opens() {
    let first = open_fake();
    close(first);
    let second = open_fake();
    assert_ne!(first, second, "a handle is never handed out twice");
    assert_eq!(call(ucdk_status, first, &[]).status, INVALID_ARGUMENT);
    assert_eq!(call(ucdk_status, second, &[]).status, OK);
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
    let mut provider = Hooked::new(move || {
        close(h.load(Ordering::SeqCst) as *mut ucdk_capi::SessionHandle);
    });
    provider.panic_on_drop = true;
    let session = into_handle(Session::with_parts(
        Box::new(provider),
        Box::new(MemoryLedger::new()),
        kit::cell(),
    ));
    handle.store(session as usize, Ordering::SeqCst);

    let reply = call(ucdk_plan, session, &plan_request(80));
    assert_eq!(reply.status, PANIC);
    assert!(error_of(&reply.bytes).message.contains("destructor"));
    assert_eq!(call(ucdk_status, session, &[]).status, INVALID_ARGUMENT);
}

/// A session over a fake that already holds a stamped object nobody records, on `ledger`;
/// returns the session and the leaked object's physical id.
fn session_with_a_leak(
    ledger: Box<dyn ucdk_ledger::Ledger>,
) -> (*mut ucdk_capi::SessionHandle, String) {
    use ucdk_provider::{Node, Provider as _, Stamp};
    let mut fake = Fake::standard();
    let path = ucdk_provider::NodePath::new("ghost/run").expect("a path");
    let node = Node::new(
        path.clone(),
        ucdk_provider::Kind::service(),
        "ghost-run".to_owned(),
        Vec::new(),
    )
    .expect("a node");
    let id = fake
        .create(&node, &Stamp::for_node(&kit::cell(), &path))
        .expect("create")
        .physical;
    let session = into_handle(Session::with_parts(Box::new(fake), ledger, kit::cell()));
    (session, id.as_str().to_owned())
}

/// Proves: `ucdk_leaks` through the library, on the in-memory and on the file ledger, answers the
/// leaks arm with the leaked object and exit class leaks (8), twice with the same answer (it
/// consumes nothing); and an apply through the same session ends done with the leak in its
/// report. Catches a verb wired to the wrong request, a response that drops the object, and a
/// sweep that differs by ledger.
#[test]
fn leaks_through_the_abi_find_the_object_on_both_ledgers() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("abi-leaks");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let file = FileLedger::open(dir.join("ledger")).expect("open");
    let ledgers: [Box<dyn ucdk_ledger::Ledger>; 2] =
        [Box::new(MemoryLedger::new()), Box::new(file)];
    for ledger in ledgers {
        let (session, ghost) = session_with_a_leak(ledger);
        let first = call(ucdk_leaks, session, &LeaksRequest {}.encode_to_vec());
        assert_eq!(first.status, OK);
        let answer = Outcome::decode(first.bytes.as_slice()).expect("an outcome");
        assert_eq!(answer.exit, ExitClass::Leaks as i32);
        let Some(outcome::Result::Leaks(found)) = &answer.result else {
            panic!("not leaks: {answer:?}");
        };
        assert_eq!(found.leaks.len(), 1);
        assert_eq!(found.leaks[0].physical_id, ghost);
        assert_eq!(found.leaks[0].owner, "ghost");
        assert_eq!(
            call(ucdk_leaks, session, &[]).bytes,
            first.bytes,
            "the empty request is the request"
        );

        let list = list_of(vec![service_of("web", 80)]);
        let planned = call(
            ucdk_plan,
            session,
            &PlanRequest {
                resources: Some(list.clone()),
                mode: Mode::Apply as i32,
            }
            .encode_to_vec(),
        );
        let hash = planned_hash(&Outcome::decode(planned.bytes.as_slice()).expect("an outcome"));
        let applied = call(
            ucdk_apply,
            session,
            &ApplyRequest {
                resources: Some(list),
                plan_hash: hash,
                rollback: false,
            }
            .encode_to_vec(),
        );
        let applied = Outcome::decode(applied.bytes.as_slice()).expect("an outcome");
        assert_eq!(applied.exit, ExitClass::Done as i32, "{applied:?}");
        let Some(outcome::Result::Done(report)) = &applied.result else {
            panic!("not done: {applied:?}");
        };
        assert_eq!(report.created.len(), 1);
        assert_eq!(report.leaks.len(), 1);
        assert_eq!(report.leaks[0].physical_id, ghost);
        close(session);
    }
}

/// Proves: the provider `fake-leaky` holds one object stamped for the session's own cell, the
/// local one and one named in the config, so the sweep reports it either way (a stamp of any
/// other cell would not be listed), and the ordinary `fake` holds none. Catches a fixture
/// stamped for a fixed cell.
#[test]
fn the_leaky_provider_leaks_into_the_sessions_own_cell() {
    for (provider, cell, leaked) in [
        ("fake-leaky", "", 1),
        ("fake-leaky", "host/cell/bbbbbbbbbbbbbbbbbbbbbbbbbb", 1),
        ("fake", "", 0),
    ] {
        let mut config = fake_config();
        config.provider = provider.to_owned();
        config.cell = cell.to_owned();
        let opened = open(&config);
        assert_eq!(opened.status, OK);
        let reply = call(ucdk_leaks, opened.session, &[]);
        let answer = Outcome::decode(reply.bytes.as_slice()).expect("an outcome");
        let Some(outcome::Result::Leaks(found)) = &answer.result else {
            panic!("not leaks: {answer:?}");
        };
        assert_eq!(found.leaks.len(), leaked, "{provider} {cell}");
        close(opened.session);
    }
}
