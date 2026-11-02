// SPDX-License-Identifier: Apache-2.0
//! The stable text id of each `Reason`: a driver may branch on it, so each id is pinned here
//! as a literal, and a new reason does not pass until it is added to the table.

use std::collections::BTreeSet;

use ucdk_release_proto::v1::Reason;

/// Every reason with the number it has on the wire and the id it has had since it was added.
const TABLE: [(Reason, i32, &str); 37] = [
    (Reason::Unspecified, 0, "UCDK-E-UNSPECIFIED"),
    (Reason::MachineFormat, 1, "UCDK-E-MACHINE-FORMAT"),
    (Reason::MachineVersion, 2, "UCDK-E-MACHINE-VERSION"),
    (Reason::StageUnknown, 3, "UCDK-E-STAGE-UNKNOWN"),
    (Reason::Selector, 4, "UCDK-E-SELECTOR"),
    (Reason::SelectorNoMatch, 5, "UCDK-E-SELECTOR-NO-MATCH"),
    (Reason::SetMember, 6, "UCDK-E-SET-MEMBER"),
    (Reason::SetHash, 7, "UCDK-E-SET-HASH"),
    (Reason::Channel, 8, "UCDK-E-CHANNEL"),
    (Reason::Credential, 9, "UCDK-E-CREDENTIAL"),
    (Reason::ApprovedNames, 10, "UCDK-E-APPROVED-NAMES"),
    (
        Reason::PublishDifferentBytes,
        11,
        "UCDK-E-PUBLISH-DIFFERENT-BYTES",
    ),
    (Reason::PublishYanked, 12, "UCDK-E-PUBLISH-YANKED"),
    (Reason::PublishReadBack, 13, "UCDK-E-PUBLISH-READ-BACK"),
    (Reason::PublishUpload, 14, "UCDK-E-PUBLISH-UPLOAD"),
    (Reason::NotOnMainline, 15, "UCDK-E-NOT-ON-MAINLINE"),
    (Reason::ExceptionalReason, 16, "UCDK-E-EXCEPTIONAL-REASON"),
    (Reason::SupersededByNewer, 17, "UCDK-E-SUPERSEDED-BY-NEWER"),
    (Reason::InputsChanged, 18, "UCDK-E-INPUTS-CHANGED"),
    (Reason::Validation, 19, "UCDK-E-VALIDATION"),
    (Reason::CannotTell, 20, "UCDK-E-CANNOT-TELL"),
    (Reason::LeaseHeld, 21, "UCDK-E-LEASE-HELD"),
    (Reason::LeaseLost, 22, "UCDK-E-LEASE-LOST"),
    (Reason::Ledger, 23, "UCDK-E-LEDGER"),
    (Reason::BuildFailed, 24, "UCDK-E-BUILD-FAILED"),
    (Reason::RowAccounting, 25, "UCDK-E-ROW-ACCOUNTING"),
    (Reason::PinMismatch, 26, "UCDK-E-PIN-MISMATCH"),
    (Reason::Source, 27, "UCDK-E-SOURCE"),
    (Reason::StageNotAdmitted, 28, "UCDK-E-STAGE-NOT-ADMITTED"),
    (Reason::RunHashMismatch, 29, "UCDK-E-RUN-HASH-MISMATCH"),
    (Reason::LeaseUnconfirmed, 30, "UCDK-E-LEASE-UNCONFIRMED"),
    (Reason::MemberVersion, 31, "UCDK-E-MEMBER-VERSION"),
    (Reason::PlatformMismatch, 32, "UCDK-E-PLATFORM-MISMATCH"),
    (Reason::BuildOutput, 33, "UCDK-E-BUILD-OUTPUT"),
    (Reason::PublishStage, 34, "UCDK-E-PUBLISH-STAGE"),
    (Reason::PublishVanished, 35, "UCDK-E-PUBLISH-VANISHED"),
    (Reason::PublishBurned, 36, "UCDK-E-PUBLISH-BURNED"),
];

#[test]
fn each_reason_has_the_id_it_has_always_had() {
    for (reason, _, id) in TABLE {
        assert_eq!(reason.id(), id, "{reason:?}");
    }
}

#[test]
fn the_table_names_every_reason_the_wire_defines() {
    let defined: Vec<Reason> = (0..256).filter_map(|n| Reason::try_from(n).ok()).collect();
    let listed: Vec<Reason> = TABLE.iter().map(|(r, _, _)| *r).collect();
    assert_eq!(listed, defined);
    for (reason, number, _) in TABLE {
        assert_eq!(reason as i32, number, "{reason:?}");
    }
}

#[test]
fn no_two_reasons_share_an_id() {
    let ids: BTreeSet<&str> = TABLE.iter().map(|(r, _, _)| r.id()).collect();
    assert_eq!(ids.len(), TABLE.len());
}

#[test]
fn an_id_is_the_prefix_and_upper_case_words_joined_by_dashes() {
    for (reason, _, _) in TABLE {
        let id = reason.id();
        let words = id
            .strip_prefix("UCDK-E-")
            .unwrap_or_else(|| panic!("{id:?}"));
        assert!(!words.is_empty(), "{id:?}");
        for word in words.split('-') {
            assert!(
                !word.is_empty()
                    && word
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit()),
                "{id:?}"
            );
        }
    }
}

#[test]
fn ids_that_kci_spelled_keep_their_words() {
    // Where kci's id (`KCI-E-<WORDS>`) had a reason of the same meaning, the words are the same.
    for (reason, words) in [
        (Reason::SetHash, "SET-HASH"),
        (Reason::StageUnknown, "STAGE-UNKNOWN"),
        (Reason::Selector, "SELECTOR"),
        (Reason::SelectorNoMatch, "SELECTOR-NO-MATCH"),
        (Reason::Channel, "CHANNEL"),
        (Reason::Credential, "CREDENTIAL"),
        (Reason::PublishDifferentBytes, "PUBLISH-DIFFERENT-BYTES"),
        (Reason::PublishUpload, "PUBLISH-UPLOAD"),
        (Reason::PublishReadBack, "PUBLISH-READ-BACK"),
        (Reason::CannotTell, "CANNOT-TELL"),
        (Reason::Validation, "VALIDATION"),
        (Reason::BuildFailed, "BUILD-FAILED"),
        (Reason::PlatformMismatch, "PLATFORM-MISMATCH"),
    ] {
        assert_eq!(reason.id(), format!("UCDK-E-{words}"));
    }
}
