// SPDX-License-Identifier: Apache-2.0
//! The self-test of the publisher kit for a registry that is wrong behind one kind of channel
//! only: a public channel addressed by group, a private one addressed by group, a private one
//! addressed by coordinate, a private one that does not stage. The channels of every other kind
//! are the conforming fake, so a scenario that runs on the wrong kind alone is held to fail the
//! stub there: a private or grouped publisher runs the lifecycle scenarios of a plain one.

mod rigs;

use rigs::publisher::{FakePublisherRig, Flaw};
use rigs::publisher_flaw::Behind;
use rigs::publisher_lax::Lax;
use rigs::publisher_more::{More, MoreRig};
use rigs::publisher_names::*;
use ucdk_release_seams::kit::{Report, run_publisher_kit};

/// A stub that is wrong behind one kind of channel.
#[derive(Debug, Clone, Copy)]
enum Wrong {
    Flaw(Flaw),
    More(More),
}

fn run_behind(wrong: Wrong, behind: Behind) -> Report {
    let base = FakePublisherRig::conforming();
    match wrong {
        Wrong::Flaw(flaw) => run_publisher_kit(&base.with_flaw_behind(flaw, behind)),
        Wrong::More(more) => run_publisher_kit(&MoreRig::flawed_behind(base, more, behind)),
    }
}

type Row = (Wrong, &'static [Behind], &'static [&'static str]);

/// Every stub with the kinds of channel it is behind and the scenarios that fail it there, in the
/// kit's order.
fn table() -> Vec<Row> {
    use Flaw as F;
    let staging: &[Behind] = &Behind::STAGING;
    let not_public_plain: &[Behind] = &Behind::NOT_PUBLIC_PLAIN;
    let private_unstaged: &[Behind] = &[Behind::PrivateUnstaged];
    vec![
        // A commit that lost a race replaces the bytes that won.
        // A second commit of the same bytes is the race of the second commit, which the scenario
        // about it runs on every channel that stages.
        (
            Wrong::Flaw(F::ARacedCommitOverwrites),
            staging,
            &[RACE, SECOND_COMMIT],
        ),
        // A commit of a set that fails leaves its first coordinate readable.
        (Wrong::Flaw(F::SetCommitIsTorn), staging, &[SET_COMMIT]),
        (
            Wrong::More(More::NonStagingSetCommitIsTorn),
            private_unstaged,
            &[SET_COMMIT, NOT_STAGING],
        ),
        (
            Wrong::More(More::NonStagingRaceOverwrites),
            private_unstaged,
            &[NOT_STAGING],
        ),
        // A stage over the bytes of another writer is staged.
        (
            Wrong::Flaw(F::StageOverOtherBytesIsStaged),
            staging,
            &[DUP_OTHER],
        ),
        (Wrong::Flaw(F::AbortDoesNothing), staging, &[ABORT]),
        // A yank by version yanks one platform, every platform of every version, or ignores the
        // grant; a yanked coordinate cannot be fetched.
        (
            Wrong::Flaw(F::VersionYankIsPerCoordinate),
            not_public_plain,
            &[YANK],
        ),
        (
            Wrong::Flaw(F::VersionYankReachesEveryVersion),
            not_public_plain,
            &[YANK],
        ),
        (
            Wrong::Flaw(F::VersionYankIgnoresTheGrant),
            not_public_plain,
            &[YANK],
        ),
        (
            Wrong::Flaw(F::YankedIsNotFetchable),
            not_public_plain,
            &[YANK],
        ),
        (
            Wrong::Flaw(F::CommitOverYankedCommits),
            staging,
            &[YANKED_COMMIT],
        ),
        (
            Wrong::More(More::NonStagingCommitOverYanked),
            private_unstaged,
            &[YANKED_COMMIT, NOT_STAGING],
        ),
        // `newest` and `list`, asked with a read grant, name what is held, with what is held
        // beside it.
        (
            Wrong::Flaw(F::NewestDropsYanked),
            not_public_plain,
            &[NEWEST],
        ),
        (
            Wrong::Flaw(F::NewestIsEmpty),
            &[Behind::PublicGrouped],
            &[PUBLIC_READ, NEWEST],
        ),
        (
            Wrong::Flaw(F::NewestIsEmpty),
            &[
                Behind::PrivateGrouped,
                Behind::PrivatePlain,
                Behind::PrivateUnstaged,
            ],
            &[NEWEST],
        ),
        (
            Wrong::Flaw(F::NewestWrongDigest),
            not_public_plain,
            &[NEWEST],
        ),
        (
            Wrong::Flaw(F::NewestDropsTheStamp),
            not_public_plain,
            &[NEWEST],
        ),
        (
            Wrong::Flaw(F::NewestDropsTheExceptionalMark),
            not_public_plain,
            &[NEWEST],
        ),
        (Wrong::Flaw(F::ListIsEmpty), not_public_plain, &[LIST]),
        (Wrong::Flaw(F::ListWrongDigest), not_public_plain, &[LIST]),
        (Wrong::Flaw(F::ListDropsTheStamp), not_public_plain, &[LIST]),
        // An observe reports the digest of the bytes sent, the stamp and the mark, of every member
        // of a set.
        (
            Wrong::Flaw(F::PresentDigestIsWrong),
            not_public_plain,
            &[COMMIT_READABLE, BECOMING, ECHO],
        ),
        (
            Wrong::Flaw(F::TheStampIsDropped),
            not_public_plain,
            &[COMMIT_READABLE, STAMP, BECOMING],
        ),
        (
            Wrong::Flaw(F::TheMarkIsDropped),
            not_public_plain,
            &[COMMIT_READABLE, STAMP],
        ),
        (
            Wrong::Flaw(F::LaterMembersHaveAWrongDigest),
            not_public_plain,
            &[COMMIT_READABLE, ECHO],
        ),
        (
            Wrong::Flaw(F::LaterMembersLoseTheirStamp),
            not_public_plain,
            &[COMMIT_READABLE],
        ),
        (
            Wrong::Flaw(F::LaterMembersLoseTheirMark),
            not_public_plain,
            &[COMMIT_READABLE],
        ),
        (
            Wrong::Flaw(F::EchoesTheDeclaredDigest),
            not_public_plain,
            &[ECHO],
        ),
        (
            Wrong::More(More::EchoesOnAMultiMemberSet),
            not_public_plain,
            &[ECHO],
        ),
        (
            Wrong::More(More::EchoesOnAYankedCoordinate),
            not_public_plain,
            &[ECHO],
        ),
        // These scenarios run on every shape that can do what they ask. A coordinate nobody wrote
        // is not an error.
        (
            Wrong::Flaw(F::AbsentIsAnError),
            not_public_plain,
            &[OBSERVE_ABSENT],
        ),
        // A stage over the bytes that are held, or that the caller staged, is a duplicate.
        (
            Wrong::Flaw(F::StageOverTheSameBytesIsStaged),
            staging,
            &[STAGE_PRESENT],
        ),
        (
            Wrong::Flaw(F::StageOverOwnStagedSameBytesIsStaged),
            staging,
            &[OWN_STAGED],
        ),
        (
            Wrong::Flaw(F::StageOverOwnStagedOverwrites),
            staging,
            &[OWN_STAGED],
        ),
        // A second commit of the same bytes is a duplicate, on a channel that stages or not.
        (
            Wrong::Flaw(F::SecondCommitOfTheSameBytesIsCommitted),
            not_public_plain,
            &[SECOND_COMMIT],
        ),
        // A failure after a commit landed is not permanent, and a commit of staged state that
        // vanished is not a commit.
        (
            Wrong::Flaw(F::TransientAfterCommitIsPermanent),
            staging,
            &[TRANSIENT],
        ),
        (
            Wrong::Flaw(F::VanishedStateIsCommitted),
            staging,
            &[VANISHED],
        ),
        // A commit repeated while the first is becoming readable is a repeat, not an error.
        (
            Wrong::Flaw(F::RetryWhileBecomingReadableIsPermanent),
            not_public_plain,
            &[BECOMING],
        ),
    ]
}

/// The stubs that are wrong about a fetch with a read grant of what is held: it finds nothing, or
/// answers the bytes of another coordinate. The read-back of a public channel is a stranger's, so
/// the scenario about the read-back is failed on a private channel only.
fn fetches() -> Vec<Row> {
    let nothing = Wrong::More(More::Lax(Lax::FetchFindsNothing));
    let another = Wrong::More(More::Lax(Lax::FetchAnswersAnother));
    let private_staging = &[Behind::PrivateGrouped, Behind::PrivatePlain];
    // A fetch that finds nothing fails every scenario that reads back what a scenario wrote with
    // one, on each channel that can do what the scenario asks.
    vec![
        (
            nothing,
            &[Behind::PublicGrouped],
            &[
                OBSERVE_ABSENT,
                COMMIT_READABLE,
                STAGE_PRESENT,
                ABORT,
                YANK,
                RACE,
                DUP_OTHER,
                SECOND_COMMIT,
                OWN_STAGED,
                COMMIT_CARRIES,
                DUPLICATE_SET,
                VANISHED,
                BECOMING,
            ],
        ),
        (
            nothing,
            private_staging,
            &[
                OBSERVE_ABSENT,
                COMMIT_READABLE,
                STAGE_PRESENT,
                ABORT,
                YANK,
                RACE,
                DUP_OTHER,
                SECOND_COMMIT,
                OWN_STAGED,
                COMMIT_CARRIES,
                DUPLICATE_SET,
                VANISHED,
                BECOMING,
                ECHO,
            ],
        ),
        (
            nothing,
            &[Behind::PrivateUnstaged],
            &[
                OBSERVE_ABSENT,
                COMMIT_READABLE,
                COMMIT_OVER,
                NOT_STAGE_BYTES,
                YANK,
                SECOND_COMMIT,
                DUPLICATE_SET,
                NOT_STAGING,
                BECOMING,
                ECHO,
            ],
        ),
        (
            another,
            &[Behind::PublicGrouped],
            &[OBSERVE_ABSENT, COMMIT_READABLE, COMMIT_CARRIES],
        ),
        (
            another,
            private_staging,
            &[OBSERVE_ABSENT, COMMIT_READABLE, COMMIT_CARRIES, ECHO],
        ),
        (
            another,
            &[Behind::PrivateUnstaged],
            &[OBSERVE_ABSENT, COMMIT_READABLE, ECHO],
        ),
    ]
}

#[test]
fn a_stub_that_is_wrong_behind_one_kind_of_channel_fails_the_scenarios_that_hold_it() {
    let rows: Vec<Row> = table().into_iter().chain(fetches()).collect();
    let runs: usize = rows.iter().map(|(_, kinds, _)| kinds.len()).sum();
    assert_eq!(runs, 137, "the number of stubs behind one kind of channel");
    let mut wrong = Vec::new();
    for (stub, kinds, named) in rows {
        for behind in kinds {
            let failed = run_behind(stub, *behind).failed();
            if failed != *named {
                wrong.push(format!(
                    "{stub:?} behind {behind:?}: failed {failed:?}, named {named:?}"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "each stub behind one kind of channel must fail exactly the scenarios it names:\n{}",
        wrong.join("\n")
    );
}
