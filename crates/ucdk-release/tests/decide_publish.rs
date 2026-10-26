// SPDX-License-Identifier: Apache-2.0
//! The decision to put one coordinate, over everything a registry can report of it.

use ucdk_release::publish::{
    Held, Presence, PublishDecision, decide_publish, decide_publish_recorded,
};
use ucdk_release::transitions::Stand;
use ucdk_release_proto::v1::{Reason, Stamped};

use PublishDecision::{AlreadyPresent, AlreadyStaged, Put, Refused};

const OURS: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn held(digest: &str) -> Held {
    Held {
        digest: digest.to_owned(),
        stamp: Some(Stamped {
            build_number: 1,
            source_commit: "c".to_owned(),
        }),
        exceptional: false,
    }
}

#[test]
fn table_over_presence_digest_yank() {
    // Every presence (absent, staged, present, yanked) with every digest a registry can report
    // beside ours: the same, another, and none exposed at all. A registry that exposes no digest
    // is never equal to ours, and a yanked coordinate is burned whatever its bytes.
    type Row = (&'static str, fn(Held) -> Presence, [PublishDecision; 3]);
    let rows: [Row; 3] = [
        (
            "staged",
            Presence::Staged,
            [
                AlreadyStaged,
                Refused(Reason::PublishDifferentBytes),
                Refused(Reason::Channel),
            ],
        ),
        (
            "present",
            Presence::Present,
            [
                AlreadyPresent,
                Refused(Reason::PublishDifferentBytes),
                Refused(Reason::Channel),
            ],
        ),
        (
            "yanked",
            Presence::Yanked,
            [Refused(Reason::PublishYanked); 3],
        ),
    ];
    let digests = [OURS, OTHER, ""];
    for (what, make, wanted) in rows {
        for (digest, want) in digests.into_iter().zip(wanted) {
            let presence = make(held(digest));
            assert_eq!(
                decide_publish(OURS, &presence),
                want,
                "{what} with digest {digest:?}"
            );
        }
    }
    // Nothing is there, so there is no digest to compare.
    assert_eq!(decide_publish(OURS, &Presence::Absent), Put);
}

#[test]
fn the_comparison_is_byte_for_byte() {
    // No case folding and no prefix: a digest that differs in any byte is another digest, for a
    // published coordinate and for a staged one.
    let upper = OURS.to_uppercase();
    for make in [Presence::Present, Presence::Staged] {
        for other in [upper.as_str(), &OURS[..63]] {
            assert_eq!(
                decide_publish(OURS, &make(held(other))),
                Refused(Reason::PublishDifferentBytes),
                "{other}"
            );
        }
    }
}

#[test]
fn only_what_is_published_counts_as_already_present() {
    // Our bytes staged are not published: the commit is still to be made.
    assert_eq!(
        decide_publish(OURS, &Presence::Staged(held(OURS))),
        AlreadyStaged
    );
    assert_ne!(
        decide_publish(OURS, &Presence::Staged(held(OURS))),
        AlreadyPresent
    );
}

#[test]
fn a_confirmed_publication_whose_coordinate_vanished_is_refused() {
    // The ledger says the bytes were committed and read back, and the registry now reports
    // nothing there, or only a staged copy: they went away out of band. They are never put
    // again behind a confirmed record, which is audit data. The refusal is its own reason, not
    // a registry that cannot be read.
    assert_eq!(
        decide_publish_recorded(OURS, Stand::Confirmed, &Presence::Absent),
        Refused(Reason::PublishVanished)
    );
    assert_eq!(
        decide_publish_recorded(OURS, Stand::Confirmed, &Presence::Staged(held(OURS))),
        Refused(Reason::PublishVanished)
    );
    // Without a confirmed record the same report is an ordinary absence: nothing was put, or an
    // intent whose effect did not land.
    for record in [Stand::Absent, Stand::Intent, Stand::Reaping] {
        assert_eq!(
            decide_publish_recorded(OURS, record, &Presence::Absent),
            Put,
            "{record:?}"
        );
    }
}

#[test]
fn the_decision_over_every_record_state_and_report() {
    // Every state of the record with every report of the registry and every digest it can
    // carry. The record changes the answer in the cells of a confirmed record over a coordinate
    // that is absent or only staged (confirmed means it was committed); every other cell is
    // `decide_publish` of the report alone, so a registry that still holds the bytes, other
    // bytes or a yanked coordinate is answered as before.
    let digests = [OURS, OTHER, ""];
    let reports = |digest: &str| {
        [
            Presence::Absent,
            Presence::Staged(held(digest)),
            Presence::Present(held(digest)),
            Presence::Yanked(held(digest)),
        ]
    };
    assert_eq!(Stand::ALL.len(), 4);
    for record in Stand::ALL {
        for digest in digests {
            for presence in reports(digest) {
                let uncommitted = matches!(presence, Presence::Absent | Presence::Staged(_));
                let want = if record == Stand::Confirmed && uncommitted {
                    Refused(Reason::PublishVanished)
                } else {
                    decide_publish(OURS, &presence)
                };
                assert_eq!(
                    decide_publish_recorded(OURS, record, &presence),
                    want,
                    "{record:?} over {presence:?}"
                );
            }
        }
    }
    // The cells that matter, spelled out, so the table above is not its own oracle.
    let confirmed = Stand::Confirmed;
    for digest in digests {
        assert_eq!(
            decide_publish_recorded(OURS, confirmed, &Presence::Staged(held(digest))),
            Refused(Reason::PublishVanished),
            "staged with {digest:?}"
        );
        assert_eq!(
            decide_publish_recorded(OURS, Stand::Intent, &Presence::Staged(held(OURS))),
            AlreadyStaged
        );
    }
    assert_eq!(
        decide_publish_recorded(OURS, confirmed, &Presence::Present(held(OURS))),
        AlreadyPresent
    );
    assert_eq!(
        decide_publish_recorded(OURS, confirmed, &Presence::Present(held(OTHER))),
        Refused(Reason::PublishDifferentBytes)
    );
    assert_eq!(
        decide_publish_recorded(OURS, confirmed, &Presence::Yanked(held(OURS))),
        Refused(Reason::PublishYanked)
    );
}
