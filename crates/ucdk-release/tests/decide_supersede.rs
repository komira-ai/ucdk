// SPDX-License-Identifier: Apache-2.0
//! The decision to stand down for a newer build, over the history a channel holds and what the
//! source says of ancestry.

use std::cell::RefCell;

use ucdk_release::publish::{Held, Presence};
use ucdk_release::supersede::{SupersedeDecision, decide_supersede};
use ucdk_release_proto::v1::{Reason, Stamped, Tri};

use SupersedeDecision::{Indeterminate, Proceed, Refused, Superseded};

fn ours() -> Stamped {
    Stamped {
        build_number: 10,
        source_commit: "ours".to_owned(),
    }
}

fn held(build: u64, commit: &str, exceptional: bool) -> Held {
    Held {
        digest: "d".repeat(64),
        stamp: Some(Stamped {
            build_number: build,
            source_commit: commit.to_owned(),
        }),
        exceptional,
    }
}

fn present(build: u64, commit: &str) -> Presence {
    Presence::Present(held(build, commit, false))
}

fn decide(history: &[Presence], answer: Tri) -> SupersedeDecision {
    decide_supersede(&ours(), history, &|_, _| answer)
}

#[test]
fn table_over_history_and_ancestry() {
    let unstamped = Presence::Present(Held {
        digest: "d".repeat(64),
        stamp: None,
        exceptional: false,
    });
    // Each kind of held build, and what each answer of the source makes of it. Builds that are
    // not newer, or not counted, never ask the source anything that matters.
    let rows: Vec<(&str, Presence, [SupersedeDecision; 4])> = vec![
        ("nothing held", Presence::Absent, [Proceed; 4]),
        ("older", present(9, "old"), [Proceed; 4]),
        ("the same build", present(10, "ours"), [Proceed; 4]),
        (
            "newer but exceptional",
            Presence::Present(held(11, "new", true)),
            [Proceed; 4],
        ),
        (
            "newer",
            present(11, "new"),
            [Superseded, Refused, Indeterminate, Indeterminate],
        ),
        (
            "newer and yanked",
            Presence::Yanked(held(11, "new", false)),
            [Superseded, Refused, Indeterminate, Indeterminate],
        ),
        (
            "equal number, another commit",
            present(10, "other"),
            [Superseded, Refused, Indeterminate, Indeterminate],
        ),
        ("no stamp", unstamped, [Indeterminate; 4]),
    ];
    // The answers, in the order of the expected decisions.
    let answers = [Tri::Yes, Tri::No, Tri::Unknown, Tri::Unspecified];
    for (what, history, wanted) in rows {
        for (answer, want) in answers.iter().zip(wanted) {
            assert_eq!(
                decide(std::slice::from_ref(&history), *answer),
                want,
                "{what}, {answer:?}"
            );
        }
    }
}

#[test]
fn equal_build_number_other_commit_supersedes() {
    assert_eq!(decide(&[present(10, "other")], Tri::Yes), Superseded);
    assert_eq!(decide(&[present(10, "ours")], Tri::Yes), Proceed);
}

#[test]
fn exceptional_builds_are_not_counted() {
    let history = [Presence::Present(held(99, "hotfix", true))];
    assert_eq!(decide(&history, Tri::Yes), Proceed);
    assert_eq!(decide(&history, Tri::No), Proceed);
    // An exceptional build that has no stamp does not make the run indeterminate either.
    let unstamped = Presence::Present(Held {
        digest: "d".repeat(64),
        stamp: None,
        exceptional: true,
    });
    assert_eq!(decide(&[unstamped], Tri::No), Proceed);
}

#[test]
fn cannot_tell_is_indeterminate() {
    assert_eq!(decide(&[present(11, "new")], Tri::Unknown), Indeterminate);
    assert_eq!(Indeterminate.reason(), Some(Reason::CannotTell));
}

#[test]
fn any_member_with_a_newer_build_decides_for_the_set() {
    // Three members: nothing held, an older build, and a newer one. The newer decides.
    let history = [Presence::Absent, present(9, "old"), present(12, "new")];
    assert_eq!(decide(&history, Tri::Yes), Superseded);
    assert_eq!(decide(&history, Tri::No), Refused);
}

#[test]
fn yanked_builds_still_count() {
    let history = [Presence::Yanked(held(12, "new", false))];
    assert_eq!(decide(&history, Tri::Yes), Superseded);
}

#[test]
fn every_newer_build_must_descend_from_ours() {
    let seen = RefCell::new(Vec::new());
    let history = [present(11, "a"), present(12, "b"), present(9, "old")];
    // Ours is an ancestor of the first newer build and not of the second.
    let decision = decide_supersede(&ours(), &history, &|mine, theirs| {
        seen.borrow_mut().push((mine.to_owned(), theirs.to_owned()));
        if theirs == "a" { Tri::Yes } else { Tri::No }
    });
    assert_eq!(decision, Refused);
    assert_eq!(
        seen.into_inner(),
        [
            ("ours".to_owned(), "a".to_owned()),
            ("ours".to_owned(), "b".to_owned())
        ]
    );
    let all_yes = decide(&history, Tri::Yes);
    assert_eq!(all_yes, Superseded);
}

#[test]
fn a_refusal_stands_even_when_another_build_cannot_be_compared() {
    let history = [present(11, "a"), present(12, "b")];
    let decision = decide_supersede(&ours(), &history, &|_, theirs| {
        if theirs == "a" { Tri::Unknown } else { Tri::No }
    });
    assert_eq!(decision, Refused);
}

#[test]
fn a_build_that_cannot_be_compared_is_indeterminate_beside_one_that_descends() {
    let history = [present(11, "a"), present(12, "b")];
    let decision = decide_supersede(&ours(), &history, &|_, theirs| {
        if theirs == "a" {
            Tri::Yes
        } else {
            Tri::Unknown
        }
    });
    assert_eq!(decision, Indeterminate);
}

#[test]
fn the_decisions_say_why() {
    assert_eq!(Proceed.reason(), None);
    assert_eq!(Superseded.reason(), Some(Reason::SupersededByNewer));
    assert_eq!(Refused.reason(), Some(Reason::SupersededByNewer));
    assert_eq!(Indeterminate.reason(), Some(Reason::CannotTell));
}
