// SPDX-License-Identifier: Apache-2.0
//! The text renderer on its own, for the values the library does not produce today.

use ucdk_api::v1::{LeaseView, LedgerRecord, LedgerShowResponse};
use ucdk_cli::ledger_text;

#[test]
fn an_unknown_state_or_origin_is_printed_as_unknown() {
    let response = LedgerShowResponse {
        token: "v1".to_owned(),
        fence: 1,
        lease: Some(LeaseView {
            holder: "ci-7".to_owned(),
            expires_ms: 5,
            // A state this build does not know, and the unspecified one.
            state: 99,
            yours: false,
        }),
        records: vec![
            LedgerRecord {
                path: "a/run".to_owned(),
                kind: "service".to_owned(),
                name: "a".to_owned(),
                origin: 99,
                state: 99,
                ..LedgerRecord::default()
            },
            LedgerRecord {
                path: "b/run".to_owned(),
                kind: "service".to_owned(),
                name: "b".to_owned(),
                // The unspecified values, which the library never writes.
                origin: 0,
                state: 0,
                ..LedgerRecord::default()
            },
        ],
    };
    assert_eq!(
        ledger_text(&response),
        "token v1\nfence 1\nlease ci-7 expires_ms 5 unknown other\n\
         record a/run service a unknown unknown physical=% deps=\n\
         record b/run service b unknown unknown physical=% deps=\n"
    );
}

#[test]
fn a_lease_state_the_library_does_know_is_named() {
    use ucdk_api::v1::LeaseState;
    let with = |state: LeaseState, yours: bool| {
        ledger_text(&LedgerShowResponse {
            lease: Some(LeaseView {
                holder: "h".to_owned(),
                expires_ms: 1,
                state: state as i32,
                yours,
            }),
            ..LedgerShowResponse::default()
        })
    };
    assert!(with(LeaseState::Live, true).contains("lease h expires_ms 1 live yours\n"));
    assert!(with(LeaseState::Lapsing, false).contains("lease h expires_ms 1 lapsing other\n"));
    assert!(with(LeaseState::Lapsed, false).contains("lease h expires_ms 1 lapsed other\n"));
    assert!(with(LeaseState::Unspecified, false).contains("lease h expires_ms 1 unknown other\n"));
}
