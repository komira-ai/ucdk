// SPDX-License-Identifier: Apache-2.0
//! The cross-SDK wire contract: one golden file per case, the bytes this crate's messages encode
//! to. Every SDK builds the same value, and its generated encoder must write the same bytes; it
//! must decode the bytes back to the same value (`sdk/python/tests/test_wire_golden.py` is the
//! Python half). The files are in `sdk/wire-golden`, with `manifest.txt` naming the message each
//! one holds (`<file> <message>`), so a golden cannot be read as the wrong message.
//!
//! The cases are chosen so that, together, every field of `ucdk.api.v1` is written at least once
//! (`ucdk-codegen/tests/golden_coverage.rs` holds them to it), including the shapes whose bytes
//! depend on the writer: a oneof written where its lowest arm sits, a message that is absent, and
//! a message with no fields.
//!
//! To change a golden on purpose, edit the case and run
//! `cargo test -p ucdk-api --test sdk_golden -- --ignored write_goldens`; the diff of the files is
//! what review reads.

use std::path::PathBuf;

use prost::Message as _;
use ucdk_api::v1::{
    Error, ErrorCode, ExitClass, LeaseState, LeaseView, LedgerRecord, LedgerShowRequest,
    LedgerShowResponse, Outcome, Pair, RecordOrigin, RecordState, SessionConfig, outcome,
};

struct Case {
    file: &'static str,
    message: &'static str,
    bytes: Vec<u8>,
}

fn case(file: &'static str, message: &'static str, value: &impl prost::Message) -> Case {
    Case {
        file,
        message,
        bytes: value.encode_to_vec(),
    }
}

fn text(s: &str) -> String {
    s.to_owned()
}

fn error(code: ErrorCode, message: &str) -> Error {
    Error {
        code: code as i32,
        message: text(message),
    }
}

/// A ledger document that sets every field of the response, with the lease as given.
fn response(lease: Option<LeaseView>) -> LedgerShowResponse {
    LedgerShowResponse {
        token: text("v7"),
        fence: 3,
        lease,
        records: vec![LedgerRecord {
            path: text("app/run"),
            kind: text("service"),
            name: text("app-run"),
            origin: RecordOrigin::Adopted as i32,
            state: RecordState::Reaping as i32,
            physical: text("run-1"),
            deps: vec![text("app/db"), text("app/cache")],
            outputs: vec![
                Pair {
                    key: text("HOST"),
                    value: text("run-host"),
                },
                Pair {
                    key: text("PORT"),
                    value: text("8080"),
                },
            ],
        }],
    }
}

fn cases() -> Vec<Case> {
    let lease = LeaseView {
        holder: text("ci-7"),
        expires_ms: 4_600_000,
        state: LeaseState::Lapsing as i32,
        yours: true,
    };
    vec![
        case(
            "session_config",
            "ucdk.api.v1.SessionConfig",
            &SessionConfig {
                ledger_kind: text("file"),
                ledger_location: text("/var/ucdk/ledger"),
                holder: text("ci-7"),
                test_clock_ms: 2_000_000,
            },
        ),
        case(
            "error",
            "ucdk.api.v1.Error",
            &error(ErrorCode::Ledger, "the ledger is corrupt"),
        ),
        // Arm 6 sits above the exit field (5) in the message but is written before it.
        case(
            "outcome_invalid",
            "ucdk.api.v1.Outcome",
            &Outcome {
                result: Some(outcome::Result::Invalid(error(
                    ErrorCode::InvalidRequest,
                    "the request is not valid",
                ))),
                exit: ExitClass::Usage as i32,
            },
        ),
        // A message with no fields is no bytes.
        case(
            "ledger_show_request",
            "ucdk.api.v1.LedgerShowRequest",
            &LedgerShowRequest {},
        ),
        case(
            "ledger_show_response",
            "ucdk.api.v1.LedgerShowResponse",
            &response(Some(lease)),
        ),
        // The lease is absent here, so that its presence is exercised both ways.
        case(
            "outcome_ledger_show",
            "ucdk.api.v1.Outcome",
            &Outcome {
                result: Some(outcome::Result::LedgerShow(response(None))),
                exit: ExitClass::Done as i32,
            },
        ),
        case(
            "outcome_error",
            "ucdk.api.v1.Outcome",
            &Outcome {
                result: Some(outcome::Result::Error(error(
                    ErrorCode::Ledger,
                    "the ledger is corrupt",
                ))),
                exit: ExitClass::FailedNoEffect as i32,
            },
        ),
    ]
}

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sdk/wire-golden")
}

fn manifest(cases: &[Case]) -> String {
    cases
        .iter()
        .map(|c| format!("{}.bin {}\n", c.file, c.message))
        .collect()
}

#[test]
fn the_golden_files_are_the_bytes_the_messages_encode_to() {
    let dir = golden_dir();
    let cases = cases();
    for c in &cases {
        let on_disk = std::fs::read(dir.join(format!("{}.bin", c.file)))
            .unwrap_or_else(|e| panic!("{}.bin is missing: {e}", c.file));
        assert_eq!(
            on_disk, c.bytes,
            "{}.bin differs from what {} encodes to now",
            c.file, c.message
        );
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("manifest.txt")).expect("manifest.txt"),
        manifest(&cases)
    );
    // No file is left over from a case that was removed.
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .expect("the golden directory")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    let mut expected: Vec<String> = cases.iter().map(|c| format!("{}.bin", c.file)).collect();
    expected.push("manifest.txt".to_owned());
    expected.sort();
    assert_eq!(files, expected);
}

#[test]
fn the_goldens_decode_back_to_messages_that_encode_to_the_same_bytes() {
    // The Rust side of the round trip: what the other SDKs are held to.
    let bytes_of = |file: &str| {
        cases()
            .into_iter()
            .find(|c| c.file == file)
            .expect("a case")
            .bytes
    };
    let shown = bytes_of("outcome_ledger_show");
    let outcome = Outcome::decode(shown.as_slice()).expect("decodes");
    assert_eq!(outcome.encode_to_vec(), shown);
    assert!(matches!(
        outcome.result,
        Some(outcome::Result::LedgerShow(LedgerShowResponse {
            lease: None,
            ..
        }))
    ));
    let empty = bytes_of("ledger_show_request");
    assert!(empty.is_empty());
    assert_eq!(
        LedgerShowRequest::decode(empty.as_slice()).expect("decodes"),
        LedgerShowRequest {}
    );
}

#[test]
#[ignore = "writes the golden files; run on purpose"]
fn write_goldens() {
    let dir = golden_dir();
    std::fs::create_dir_all(&dir).expect("create the golden directory");
    let cases = cases();
    for old in std::fs::read_dir(&dir).expect("the golden directory") {
        std::fs::remove_file(old.expect("entry").path()).expect("remove an old golden");
    }
    for c in &cases {
        std::fs::write(dir.join(format!("{}.bin", c.file)), &c.bytes).expect("write a golden");
    }
    std::fs::write(dir.join("manifest.txt"), manifest(&cases)).expect("write the manifest");
}
