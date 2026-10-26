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
    // The two are unrelated.
    assert_eq!(decide_with_directions(Tri::No, Tri::No, "other"), Refused);
    // The same commit is not another commit.
    assert_eq!(decide(&[present(10, "ours")], Tri::Yes), Proceed);
}

#[test]
fn equal_build_number_descendant_proceeds() {
    // A commit that changes nothing a build reads keeps its parent's number: the held build
    // is the parent of ours, so ours is not stood down for it.
    assert_eq!(decide_with_directions(Tri::No, Tri::Yes, "parent"), Proceed);
    // The held build was made from an ancestor further back.
    let history = [present(10, "grandparent"), present(10, "parent")];
    let decision = decide_supersede(&ours(), &history, &|mine, _| {
        if mine == "ours" { Tri::No } else { Tri::Yes }
    });
    assert_eq!(decision, Proceed);
}

/// What `ancestry` is asked, and answers, for a build of our number: `forward` for "is ours an
/// ancestor of theirs", `backward` for the other direction.
fn decide_with_directions(forward: Tri, backward: Tri, theirs: &str) -> SupersedeDecision {
    decide_supersede(&ours(), &[present(10, theirs)], &|mine, _| {
        if mine == "ours" { forward } else { backward }
    })
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
