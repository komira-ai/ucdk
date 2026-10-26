// SPDX-License-Identifier: Apache-2.0
//! The decision to put one coordinate, over everything a registry can report of it.

use ucdk_release::publish::{Held, Presence, PublishDecision, decide_publish};
use ucdk_release_proto::v1::{Reason, Stamped};

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
    use PublishDecision::{AlreadyPresent, Put, Refused};
    // Every presence, with every digest a registry can report beside ours: the same, another,
    // and none exposed at all. A registry that exposes no digest is never equal to ours.
    let table = [
        (Presence::Absent, Put),
        (Presence::Present(held(OURS)), AlreadyPresent),
        (
            Presence::Present(held(OTHER)),
            Refused(Reason::PublishDifferentBytes),
        ),
        (Presence::Present(held("")), Refused(Reason::Channel)),
        // A yanked coordinate is burned, whatever its bytes.
        (Presence::Yanked(held(OURS)), Refused(Reason::PublishYanked)),
        (
            Presence::Yanked(held(OTHER)),
            Refused(Reason::PublishYanked),
        ),
        (Presence::Yanked(held("")), Refused(Reason::PublishYanked)),
    ];
    for (presence, wanted) in table {
        assert_eq!(decide_publish(OURS, &presence), wanted, "{presence:?}");
    }
}

#[test]
fn the_comparison_is_byte_for_byte() {
    // No case folding and no prefix: a digest that differs in any byte is another digest.
    let upper = OURS.to_uppercase();
    assert_eq!(
        decide_publish(OURS, &Presence::Present(held(&upper))),
        PublishDecision::Refused(Reason::PublishDifferentBytes)
    );
    assert_eq!(
        decide_publish(OURS, &Presence::Present(held(&OURS[..63]))),
        PublishDecision::Refused(Reason::PublishDifferentBytes)
    );
}
