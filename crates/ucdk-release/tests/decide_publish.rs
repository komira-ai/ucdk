// SPDX-License-Identifier: Apache-2.0
//! The decision to put one coordinate, over everything a registry can report of it.

use ucdk_release::publish::{Held, Presence, PublishDecision, decide_publish};
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
