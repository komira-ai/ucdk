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

/// What `ancestry` is asked, and answers, for a build of our number: `forward` for "is ours an
/// ancestor of theirs", `backward` for "is theirs an ancestor of ours".
fn decide_with_directions(forward: Tri, backward: Tri, theirs: &str) -> SupersedeDecision {
    decide_supersede(&ours(), &[present(10, theirs)], &|mine, _| {
        if mine == "ours" { forward } else { backward }
    })
}

#[test]
fn table_over_history_and_ancestry() {
    let unstamped = Presence::Present(Held {
        digest: "d".repeat(64),
        stamp: None,
        exceptional: false,
    });
    // Each kind of held build, and what each answer of the source makes of it. Builds that are
    // not newer, or not counted, never decide anything whatever the source says.
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
            "newer but only staged",
            Presence::Staged(held(11, "new", false)),
            [Proceed; 4],
        ),
        // A burned number holds no build: it is not counted, whatever the source says, and its
        // lack of a stamp is not an unstamped build.
        ("burned", Presence::Burned, [Proceed; 4]),
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
    // Our commit is an ancestor of the held one, which is not an ancestor of ours.
    assert_eq!(
        decide_with_directions(Tri::Yes, Tri::No, "other"),
        Superseded
    );
    // The same commit is not another commit.
    assert_eq!(decide(&[present(10, "ours")], Tri::Yes), Proceed);
    // The two are unrelated: a refusal, not a stand-down.
    assert_eq!(decide_with_directions(Tri::No, Tri::No, "other"), Refused);
}

#[test]
fn equal_build_number_descendant_proceeds() {
    // A commit that changes nothing a build reads keeps its parent's number: the held build is
    // the parent of ours, so ours is not stood down for it.
    assert_eq!(decide_with_directions(Tri::No, Tri::Yes, "parent"), Proceed);
    // The held builds were made from ancestors further back.
    let history = [present(10, "grandparent"), present(10, "parent")];
    let decision = decide_supersede(&ours(), &history, &|mine, _| {
        if mine == "ours" { Tri::No } else { Tri::Yes }
    });
    assert_eq!(decision, Proceed);
}

#[test]
fn equal_build_number_asks_whether_theirs_is_an_ancestor_first() {
    let seen = RefCell::new(Vec::new());
    let ask = |a: &str, b: &str| {
        seen.borrow_mut().push((a.to_owned(), b.to_owned()));
        Tri::No
    };
    let decision = decide_supersede(&ours(), &[present(10, "other")], &ask);
    assert_eq!(decision, Refused);
    assert_eq!(
        seen.take(),
        [
            ("other".to_owned(), "ours".to_owned()),
            ("ours".to_owned(), "other".to_owned())
        ]
    );
    // Once theirs is known to be an ancestor of ours, the second question is not asked.
    let decision = decide_supersede(&ours(), &[present(10, "other")], &|a, b| {
        seen.borrow_mut().push((a.to_owned(), b.to_owned()));
        Tri::Yes
    });
    assert_eq!(decision, Proceed);
    assert_eq!(seen.take(), [("other".to_owned(), "ours".to_owned())]);
    // A greater build number is one question, ours against theirs.
    decide_supersede(&ours(), &[present(11, "new")], &|a, b| {
        seen.borrow_mut().push((a.to_owned(), b.to_owned()));
        Tri::Yes
    });
    assert_eq!(seen.take(), [("ours".to_owned(), "new".to_owned())]);
}

#[test]
fn equal_build_number_over_every_pair_of_answers() {
    // Rows are the answer to "is ours an ancestor of theirs"; columns the answer to "is theirs
    // an ancestor of ours": yes, no, unknown, unspecified. A held build that is our ancestor
    // never stops the run; a forward yes always does; otherwise only a definite no from both
    // refuses, and anything the source cannot tell is indeterminate.
    let answers = [Tri::Yes, Tri::No, Tri::Unknown, Tri::Unspecified];
    let table = [
        (Tri::Yes, [Proceed, Superseded, Superseded, Superseded]),
        (Tri::No, [Proceed, Refused, Indeterminate, Indeterminate]),
        (
            Tri::Unknown,
            [Proceed, Indeterminate, Indeterminate, Indeterminate],
        ),
        (
            Tri::Unspecified,
            [Proceed, Indeterminate, Indeterminate, Indeterminate],
        ),
    ];
    for ((forward, row), asked) in table.into_iter().zip(answers) {
        assert_eq!(forward, asked);
        for (backward, want) in answers.into_iter().zip(row) {
            assert_eq!(
                decide_with_directions(forward, backward, "other"),
                want,
                "forward {forward:?}, backward {backward:?}"
            );
        }
    }
}

#[test]
fn exceptional_builds_are_not_counted() {
    let history = [Presence::Present(held(99, "hotfix", true))];
    assert_eq!(decide(&history, Tri::Yes), Proceed);
    assert_eq!(decide(&history, Tri::No), Proceed);
    // An exceptional yanked build is not counted either.
    let yanked = [Presence::Yanked(held(99, "hotfix", true))];
    assert_eq!(decide(&yanked, Tri::No), Proceed);
    // An exceptional build that has no stamp does not make the run indeterminate.
    let unstamped = Presence::Present(Held {
        digest: "d".repeat(64),
        stamp: None,
        exceptional: true,
    });
    assert_eq!(decide(&[unstamped], Tri::No), Proceed);
    // Beside a build that is counted, it changes nothing.
    let mixed = [
        Presence::Present(held(99, "hotfix", true)),
        present(11, "new"),
    ];
    assert_eq!(decide(&mixed, Tri::Yes), Superseded);
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
fn a_burned_member_is_not_a_held_build_and_does_not_turn_the_set_indeterminate() {
    // A burned member has no stamp, but nothing was built there: the set is decided by the
    // builds that exist, as if the burned member held nothing.
    for answer in [Tri::Yes, Tri::No, Tri::Unknown] {
        let with = [Presence::Burned, present(12, "new")];
        let without = [Presence::Absent, present(12, "new")];
        assert_eq!(
            decide(&with, answer),
            decide(&without, answer),
            "{answer:?}"
        );
    }
    assert_eq!(decide(&[Presence::Burned], Tri::No), Proceed);
    assert_eq!(
        decide(&[Presence::Burned, present(12, "new")], Tri::Yes),
        Superseded
    );
}

#[test]
fn yanked_builds_still_count() {
    let history = [Presence::Yanked(held(12, "new", false))];
    assert_eq!(decide(&history, Tri::Yes), Superseded);
    assert_eq!(decide(&history, Tri::No), Refused);
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
    assert_eq!(decide(&history, Tri::Yes), Superseded);
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
fn a_build_without_a_stamp_is_indeterminate_beside_one_that_descends() {
    let unstamped = Presence::Present(Held {
        digest: "d".repeat(64),
        stamp: None,
        exceptional: false,
    });
    assert_eq!(
        decide(&[present(11, "a"), unstamped.clone()], Tri::Yes),
        Indeterminate
    );
    // A refusal outranks it.
    assert_eq!(decide(&[present(11, "a"), unstamped], Tri::No), Refused);
}

#[test]
fn the_decisions_say_why() {
    assert_eq!(Proceed.reason(), None);
    assert_eq!(Superseded.reason(), Some(Reason::SupersededByNewer));
    assert_eq!(Refused.reason(), Some(Reason::SupersededByNewer));
    assert_eq!(Indeterminate.reason(), Some(Reason::CannotTell));
}
