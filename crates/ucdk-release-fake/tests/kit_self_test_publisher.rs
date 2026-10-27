// SPDX-License-Identifier: Apache-2.0
//! The self-test of the publisher kit: for every scenario a stub of the registry that breaks the
//! promise it holds and is failed by it, further stubs, each a wrong implementation that fails
//! exactly the scenarios its row names, the declarations a rig can get wrong, a rig that can build
//! one kind of channel only, which must skip exactly the scenarios that need another and fail
//! every stub of the ones it still runs, and a rig of public channels only, which must skip the
//! scenarios about private and grouped ones.

mod rigs;

use rigs::publisher::{FakePublisherRig, Flaw};
use rigs::publisher_flaw::{Bad, Behind, Call, Hook};
use rigs::publisher_lax::{
    Defect, Holder, Kind, Lax, Verb, coverage_of_two, field_blind, on_shapes,
    others_committed_blind, others_staged_seen, partial_coverage, read_grant_writes,
    read_grants_see_staged, strangers_let_in, writer_grants_on_reads, writers_see_staged,
};
use rigs::publisher_more::{Field, More, MoreRig};
use rigs::publisher_names::*;
use rigs::{further_stubs_fail_exactly, names};
use ucdk_release_seams::kit::{ChannelShape, Report, publisher_scenarios, run_publisher_kit};

/// A wrong rig that is not one registry flaw: a declaration that is not true, or a registry
/// that does what the rig says it does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Odd {
    /// The rig says a version is sealed, and the registry does not seal it.
    DeclaresSealsOnly,
    /// The registry seals a version and the rig does not say so.
    SealsUndeclared,
    /// A sealing registry answers `Duplicate` to a later platform of a version.
    SealedIsDuplicate,
    /// The same, on a channel whose commit carries the bytes: the refusal is the commit's.
    SealedIsDuplicateOnOneCall,
    /// A rig that declares no shape of channel.
    NoShapes,
    /// A sealing rig whose only sibling is a later platform, which the registry cannot take
    /// beside the base.
    OnlyAPlatformSibling,
}

#[derive(Debug, Clone, Copy)]
enum Stub {
    Flaw(Flaw),
    More(More),
    Hook(Hook),
    Odd(Odd),
    /// The stub behind a rig that names no sibling of a coordinate.
    Siblingless(More),
}

fn run_on(base: FakePublisherRig, stub: Stub) -> Report {
    match stub {
        Stub::Flaw(flaw) => run_publisher_kit(&base.with_flaw(flaw)),
        Stub::More(more) => run_publisher_kit(&MoreRig::flawed_on(base, more)),
        Stub::Hook(hook) => run_publisher_kit(&base.hooked(hook)),
        Stub::Odd(Odd::DeclaresSealsOnly) => {
            run_publisher_kit(&FakePublisherRig::declaring_seals_only())
        }
        Stub::Odd(Odd::SealsUndeclared) => {
            run_publisher_kit(&FakePublisherRig::sealing_undeclared())
        }
        Stub::Odd(Odd::SealedIsDuplicate) => run_publisher_kit(
            &FakePublisherRig::sealing().with_flaw(Flaw::SealedVersionIsDuplicate),
        ),
        Stub::Odd(Odd::SealedIsDuplicateOnOneCall) => run_publisher_kit(
            &FakePublisherRig::sealing()
                .with_shapes(vec![ChannelShape::without_staging()])
                .with_flaw(Flaw::SealedVersionIsDuplicate),
        ),
        Stub::Odd(Odd::NoShapes) => run_publisher_kit(&base.with_shapes(Vec::new())),
        Stub::Odd(Odd::OnlyAPlatformSibling) => run_publisher_kit(
            &MoreRig::flawed_on(FakePublisherRig::sealing(), More::Conforming)
                .naming_only_a_platform(),
        ),
        Stub::Siblingless(more) => {
            run_publisher_kit(&MoreRig::flawed_on(base, more).naming_no_sibling())
        }
    }
}

fn run(stub: Stub) -> Report {
    run_on(FakePublisherRig::conforming(), stub)
}

/// The stub behind the channels of one kind only, on a rig that declares every shape: the channels
/// of the other kinds are the conforming fake.
fn run_behind(stub: Stub, behind: Behind) -> Report {
    let base = FakePublisherRig::conforming();
    match stub {
        Stub::Flaw(flaw) => run_publisher_kit(&base.with_flaw_behind(flaw, behind)),
        Stub::More(more) => run_publisher_kit(&MoreRig::flawed_behind(base, more, behind)),
        other => panic!("{other:?} is not a stub of one channel"),
    }
}

/// The stub behind the channels that do not stage only, on a rig that declares both shapes: the
/// channel that stages is the conforming fake.
fn run_behind_the_channel_that_does_not_stage(stub: Stub) -> Report {
    run_behind(stub, Behind::Unstaged)
}

fn own(scenario: &'static str, stub: Stub) -> (&'static str, Stub) {
    (scenario, stub)
}

/// The stub of each scenario, in the kit's order.
fn own_stubs() -> Vec<(&'static str, Stub)> {
    use Flaw as F;
    vec![
        own(OBSERVE_ABSENT, Stub::Flaw(F::AbsentIsAnError)),
        own(STAGE_OBSERVE, Stub::Flaw(F::StagedDigestIsWrong)),
        own(STRANGER_STAGED, Stub::Flaw(F::StrangerSeesStaged)),
        own(STAGE_GRANT, Stub::Flaw(F::StageGrantIsBlindToStaged)),
        own(COMMIT_GRANT, Stub::Flaw(F::CommitGrantIsBlindToStaged)),
        own(COMMIT_READABLE, Stub::Flaw(F::PresentDigestIsWrong)),
        own(STAGE_PRESENT, Stub::Flaw(F::StageOverTheSameBytesIsStaged)),
        own(COMMIT_OVER, Stub::Flaw(F::CommitOverOtherBytesOverwrites)),
        own(ABORT, Stub::Flaw(F::AbortDoesNothing)),
        own(SET_COMMIT, Stub::Flaw(F::SetCommitIsTorn)),
        own(NOT_STAGE_BYTES, Stub::Flaw(F::StagesLies)),
        own(NO_DIGEST, Stub::Flaw(F::OthersHaveNoDigest)),
        own(YANK, Stub::Flaw(F::YankedIsNotFetchable)),
        own(YANKED_COMMIT, Stub::Flaw(F::CommitOverYankedCommits)),
        own(LOST, Stub::Flaw(F::ObserveAfterALostAnswerIsAbsent)),
        own(LIST, Stub::Flaw(F::ObserveReadsTheLaggingList)),
        own(PUBLIC_READ, Stub::Flaw(F::AnonymousFetchIsDenied)),
        own(FOREIGN, Stub::Flaw(F::AForeignGrantIsAccepted)),
        own(PHASE, Stub::Flaw(F::TheWrongPhaseIsAccepted)),
        own(EXPIRED, Stub::Flaw(F::AnExpiredGrantIsAccepted)),
        own(NEWEST, Stub::Flaw(F::NewestDropsYanked)),
        own(STAMP, Stub::Flaw(F::TheStampIsDropped)),
        own(OTHERS_OBSERVE, Stub::Flaw(F::GrantedObserveIsBlindToOthers)),
        own(GROUP_THEN_INDEX, Stub::Flaw(F::IndexIsReadBeforeTheGroup)),
        own(ANOTHER_GROUP, Stub::Flaw(F::GroupedObserveIsBlindToOthers)),
        own(FAILED_READ, Stub::Flaw(F::FailedGroupReadIsAbsent)),
        own(DECLARED_SET, Stub::Flaw(F::TheDeclaredSetIsDropped)),
        own(EMPTY_SET, Stub::Flaw(F::AnEmptySetSeesStagedState)),
        own(READ_GRANT, Stub::Flaw(F::AReadGrantMayWrite)),
        own(RACE, Stub::Flaw(F::ARacedCommitOverwrites)),
        own(TRANSIENT, Stub::Flaw(F::TransientAfterCommitIsPermanent)),
        own(DUP_OTHER, Stub::Flaw(F::StageOverOtherBytesIsStaged)),
        own(
            SECOND_COMMIT,
            Stub::Flaw(F::SecondCommitOfTheSameBytesIsCommitted),
        ),
        own(
            OWN_STAGED,
            Stub::Flaw(F::StageOverOwnStagedSameBytesIsStaged),
        ),
        own(COMMIT_CARRIES, Stub::More(More::CommitReplacesStaged)),
        own(DUPLICATE_SET, Stub::More(More::CommitPartial)),
        own(NAMED_TWICE, Stub::More(More::DupStageDeduped)),
        own(
            WRITER_READS,
            Stub::More(More::FetchWithWriterGrantSeesStaged),
        ),
        own(NOT_STAGING, Stub::More(More::NonStagingRaceOverwrites)),
        own(VANISHED, Stub::Flaw(F::VanishedStateIsCommitted)),
        own(
            BECOMING,
            Stub::Flaw(F::RetryWhileBecomingReadableIsPermanent),
        ),
        own(
            UNREPRESENTABLE,
            Stub::Flaw(F::UnrepresentableObserveIsAbsent),
        ),
        own(SEALED, Stub::Odd(Odd::DeclaresSealsOnly)),
        own(BURNED, Stub::Flaw(F::BurnedReadsAsAbsent)),
        own(ECHO, Stub::Flaw(F::EchoesTheDeclaredDigest)),
        own(DECLARES, Stub::Flaw(F::GrantLimitIsZero)),
    ]
}

/// Every write that takes a bad grant for a good one, in every way a grant is bad.
fn every_bad_grant_taken_by_a_call() -> Vec<(Call, Bad)> {
    let mut all = Vec::new();
    for bad in [Bad::Foreign, Bad::Expired, Bad::Uncovered] {
        for call in [Call::Stage, Call::Commit, Call::Abort, Call::Yank] {
            all.push((call, bad));
        }
    }
    all
}

/// The scenarios that fail a stub that is lax about a grant on one verb, field or shape of
/// channel: the one about the defect of the grant, or the one about a read grant for a read.
fn held_by_the_lax_stub(stub: More) -> &'static [&'static str] {
    match stub {
        More::GrantBlindToField(..) | More::CoverageOfTwo(..) => &[PHASE],
        // A read grant taken for a write grant is refused on every shape the kit builds, and on a
        // private channel the scenario about read grants holds it too.
        More::ReadGrantWrites(Kind::Public, _) => &[PHASE],
        More::ReadGrantWrites(..) => &[PHASE, READ_GRANT],
        More::WriterSeesStaged(..) | More::ReadSeesStaged(_) => &[WRITER_READS],
        More::ReadSeesOthersStaged => &[DUP_OTHER],
        // A grant that names some of the coordinates of a call is refused for each of them.
        More::Lax(Lax::PartialCoverage(..)) => &[PHASE],
        // The staged state of another writer is read by no grant of any phase, in any way.
        More::Lax(Lax::OthersStagedSeen(..)) => &[DUP_OTHER],
        // An observe of a coordinate another writer committed is told the set on a channel
        // addressed by group by the scenario about another group, and is told none by the one
        // about a grant that observes another writer's commit; a channel that is not addressed
        // by group is asked both ways by the second. The scenario about a listing that lags
        // reads such a coordinate of a private channel with a read grant, and the one about an
        // empty set reads one with a stage grant.
        More::Lax(Lax::OthersCommittedBlind(Kind::Grouped, _, true)) => &[ANOTHER_GROUP],
        More::Lax(Lax::OthersCommittedBlind(Kind::Grouped, Holder::Stage, false)) => {
            &[OTHERS_OBSERVE, EMPTY_SET]
        }
        More::Lax(Lax::OthersCommittedBlind(Kind::Public, ..)) => &[OTHERS_OBSERVE],
        // A read grant that is blind to what another writer committed is blind to it when the
        // scenarios that observe a coordinate another writer holds read it with one: the one about
        // the digest of such a coordinate and the one about a set that meets it.
        More::Lax(Lax::OthersCommittedBlind(_, Holder::Read, false)) => {
            &[NO_DIGEST, LIST, OTHERS_OBSERVE, DUPLICATE_SET]
        }
        More::Lax(Lax::OthersCommittedBlind(..)) => &[OTHERS_OBSERVE],
        // What a private channel lets a stage or a commit grant read, and a stranger, is held with
        // the reads of a read grant; what a channel that is not private lets a stage or a commit
        // grant read is held with the reads that need no grant.
        More::Lax(Lax::WriterGrantOnRead(Kind::Grouped | Kind::NonStaging, ..)) => {
            &[PUBLIC_READ, READ_GRANT]
        }
        // A stranger of a private channel is refused by the scenario about a stranger and a staged
        // coordinate, as well as by the one about read grants.
        More::Lax(Lax::StrangerSeesStaged(_)) => &[STRANGER_STAGED, READ_GRANT],
        More::Lax(_) => &[READ_GRANT],
        More::BadGrantTakenOnShape(kind, defect, verb) => match (verb, defect) {
            // A grant that names the base and not the coordinate read is also held by the
            // scenario about the fields of a coordinate, which reads the sibling of a base of a
            // private channel. A bad grant on a read is refused by a public channel, which a read
            // needs no grant of, and by a private one.
            (Verb::Observe | Verb::Fetch, Defect::Uncovered) => match kind {
                Kind::Public => &[PUBLIC_READ],
                Kind::Private => &[PHASE, READ_GRANT],
                Kind::Grouped | Kind::NonStaging => &[PUBLIC_READ, PHASE, READ_GRANT],
            },
            (Verb::Observe | Verb::Fetch | Verb::Newest | Verb::List, _) => match kind {
                Kind::Public => &[PUBLIC_READ],
                Kind::Private => &[READ_GRANT],
                Kind::Grouped | Kind::NonStaging => &[PUBLIC_READ, READ_GRANT],
            },
            (_, Defect::Foreign) => &[FOREIGN],
            (_, Defect::Expired) => &[EXPIRED],
            (_, Defect::Uncovered | Defect::OtherWritePhase) => &[PHASE],
        },
        other => panic!("{other:?} is not lax about a grant on a shape"),
    }
}

/// Every stub with every scenario it fails, in the kit's order: the stub of each scenario, and
/// the further stubs.
fn every_stub() -> Vec<(Stub, &'static [&'static str])> {
    use Flaw as F;
    let mut table: Vec<(Stub, &'static [&'static str])> = vec![
        (Stub::Flaw(F::AbsentIsAnError), &[OBSERVE_ABSENT]),
        (Stub::Flaw(F::StagedDigestIsWrong), &[STAGE_OBSERVE, ECHO]),
        // A stranger told the declared set is asked too, so the scenario about reads that need no
        // grant also fails a stranger who sees staged state.
        (
            Stub::Flaw(F::StrangerSeesStaged),
            &[STRANGER_STAGED, PUBLIC_READ],
        ),
        (Stub::Flaw(F::StageGrantIsBlindToStaged), &[STAGE_GRANT]),
        (Stub::Flaw(F::CommitGrantIsBlindToStaged), &[COMMIT_GRANT]),
        (
            Stub::Flaw(F::PresentDigestIsWrong),
            &[COMMIT_READABLE, BECOMING, ECHO],
        ),
        (
            Stub::Flaw(F::StageOverTheSameBytesIsStaged),
            &[STAGE_PRESENT],
        ),
        (
            Stub::Flaw(F::CommitOverOtherBytesOverwrites),
            &[COMMIT_OVER],
        ),
        (Stub::Flaw(F::AbortDoesNothing), &[ABORT]),
        (Stub::Flaw(F::SetCommitIsTorn), &[SET_COMMIT]),
        (
            Stub::Flaw(F::StagesLies),
            &[
                OBSERVE_ABSENT,
                COMMIT_READABLE,
                COMMIT_OVER,
                SET_COMMIT,
                NOT_STAGE_BYTES,
                NO_DIGEST,
                YANK,
                YANKED_COMMIT,
                LOST,
                LIST,
                PUBLIC_READ,
                FOREIGN,
                PHASE,
                EXPIRED,
                NEWEST,
                STAMP,
                OTHERS_OBSERVE,
                READ_GRANT,
                SECOND_COMMIT,
                DUPLICATE_SET,
                NAMED_TWICE,
                NOT_STAGING,
                BECOMING,
                UNREPRESENTABLE,
                BURNED,
                ECHO,
                DECLARES,
            ],
        ),
        (Stub::Flaw(F::OthersHaveNoDigest), &[NO_DIGEST, LIST]),
        (Stub::Flaw(F::YankedIsNotFetchable), &[YANK]),
        (Stub::Flaw(F::CommitOverYankedCommits), &[YANKED_COMMIT]),
        (Stub::Flaw(F::ObserveAfterALostAnswerIsAbsent), &[LOST]),
        (Stub::Flaw(F::ObserveReadsTheLaggingList), &[LIST]),
        (Stub::Flaw(F::AnonymousFetchIsDenied), &[PUBLIC_READ, ECHO]),
        (Stub::Flaw(F::AForeignGrantIsAccepted), &[FOREIGN]),
        (Stub::Flaw(F::TheWrongPhaseIsAccepted), &[PHASE]),
        (Stub::Flaw(F::AnExpiredGrantIsAccepted), &[EXPIRED]),
        (Stub::Flaw(F::NewestDropsYanked), &[NEWEST]),
        (
            Stub::Flaw(F::TheStampIsDropped),
            &[COMMIT_READABLE, STAMP, BECOMING],
        ),
        // A read grant that is blind to a coordinate another writer committed is blind on a
        // private channel too, which the scenarios about the digest of such a coordinate, a
        // listing that lags, a race, a second commit and a set that meets it read with one.
        (
            Stub::Flaw(F::GrantedObserveIsBlindToOthers),
            &[
                NO_DIGEST,
                LIST,
                OTHERS_OBSERVE,
                RACE,
                SECOND_COMMIT,
                DUPLICATE_SET,
            ],
        ),
        (
            Stub::Flaw(F::IndexIsReadBeforeTheGroup),
            &[GROUP_THEN_INDEX],
        ),
        (
            Stub::Flaw(F::GroupedObserveIsBlindToOthers),
            &[ANOTHER_GROUP],
        ),
        (Stub::Flaw(F::FailedGroupReadIsAbsent), &[FAILED_READ]),
        // The scenario about what a grant reads of staged state runs on the channel addressed by
        // group too: its observe that is told the set, and its observe with a read grant that is
        // told none, are the two promises these stubs break.
        // A channel addressed by group finds staged state only by the set it was staged in, so
        // each scenario that observes staged state on one is told it, and fails the stub that
        // drops it.
        (
            Stub::Flaw(F::TheDeclaredSetIsDropped),
            &[
                STAGE_OBSERVE,
                STAGE_GRANT,
                COMMIT_GRANT,
                ABORT,
                SET_COMMIT,
                LOST,
                PHASE,
                STAMP,
                DECLARED_SET,
                COMMIT_CARRIES,
                DUPLICATE_SET,
                NAMED_TWICE,
                WRITER_READS,
                VANISHED,
                ECHO,
            ],
        ),
        (
            Stub::Flaw(F::AnEmptySetSeesStagedState),
            &[EMPTY_SET, WRITER_READS],
        ),
        (Stub::Flaw(F::AReadGrantMayWrite), &[PHASE, READ_GRANT]),
        (
            Stub::Flaw(F::ARacedCommitOverwrites),
            &[RACE, SECOND_COMMIT],
        ),
        (Stub::Flaw(F::TransientAfterCommitIsPermanent), &[TRANSIENT]),
        (Stub::Flaw(F::StageOverOtherBytesIsStaged), &[DUP_OTHER]),
        (
            Stub::Flaw(F::StrangerObservesPrivate),
            &[STRANGER_STAGED, READ_GRANT],
        ),
        (Stub::Flaw(F::StrangerFetchesPrivate), &[READ_GRANT]),
        (Stub::Flaw(F::StrangerNewestPrivate), &[READ_GRANT]),
        (Stub::Flaw(F::StrangerListsPrivate), &[READ_GRANT]),
        (
            Stub::Flaw(F::StrangerObservesPrivateWithASet),
            &[READ_GRANT],
        ),
        // The scenario about read grants asks a private channel for a stranger's fetch of a staged
        // coordinate too.
        (
            Stub::Flaw(F::StrangerFetchesStaged),
            &[PUBLIC_READ, READ_GRANT],
        ),
        (
            Stub::Flaw(F::ListIncludesStaged),
            &[PUBLIC_READ, WRITER_READS],
        ),
        (
            Stub::Flaw(F::NewestIncludesStaged),
            &[PUBLIC_READ, WRITER_READS],
        ),
        (Stub::Flaw(F::NewestMatchesByPrefix), &[NEWEST]),
        (Stub::Flaw(F::NewestIgnoresTheChannel), &[NEWEST]),
        (Stub::Flaw(F::NewestDropsTheStamp), &[NEWEST]),
        (Stub::Flaw(F::NewestWrongDigest), &[NEWEST]),
        (Stub::Flaw(F::NewestDropsTheExceptionalMark), &[NEWEST]),
        (Stub::Flaw(F::YankIgnoresThePhase), &[PHASE]),
        (Stub::Flaw(F::StageIgnoresCoverage), &[PHASE]),
        (Stub::Flaw(F::CommitIgnoresCoverage), &[PHASE]),
        (Stub::Flaw(F::OnlyTheLastCoordinateIsCovered), &[PHASE]),
        (Stub::Flaw(F::StrangerMayStage), &[PHASE]),
        (Stub::Flaw(F::StrangerMayCommit), &[PHASE]),
        (Stub::Flaw(F::StrangerMayAbort), &[PHASE]),
        (Stub::Flaw(F::StrangerMayYank), &[PHASE]),
        (Stub::Flaw(F::PrivateTakesAForeignReadGrant), &[READ_GRANT]),
        (Stub::Flaw(F::PrivateTakesAnExpiredReadGrant), &[READ_GRANT]),
        (
            Stub::Flaw(F::PrivateTakesAnUncoveredReadGrant),
            &[PHASE, READ_GRANT],
        ),
        (Stub::Flaw(F::ObserveDropsAForeignSet), &[DECLARED_SET]),
        (
            Stub::Flaw(F::SecondCommitOfTheSameBytesIsCommitted),
            &[SECOND_COMMIT],
        ),
        (
            Stub::Flaw(F::StageOverOwnStagedSameBytesIsStaged),
            &[OWN_STAGED],
        ),
        (Stub::Flaw(F::StageOverOwnStagedOverwrites), &[OWN_STAGED]),
        (Stub::Flaw(F::AbortGivesBackAllStaged), &[ABORT]),
        (Stub::Flaw(F::ObserveIgnoresVersion), &[OBSERVE_ABSENT]),
        (Stub::Flaw(F::ObserveIgnoresPlatform), &[OBSERVE_ABSENT]),
        (Stub::Flaw(F::ObserveIgnoresChannel), &[OBSERVE_ABSENT]),
        (
            Stub::Flaw(F::FetchIgnoresVersion),
            &[OBSERVE_ABSENT, COMMIT_CARRIES],
        ),
        (
            Stub::Flaw(F::FetchIgnoresPlatform),
            &[OBSERVE_ABSENT, COMMIT_CARRIES],
        ),
        (
            Stub::Flaw(F::FetchIgnoresChannel),
            &[OBSERVE_ABSENT, COMMIT_CARRIES],
        ),
        (Stub::Flaw(F::ListDropsTheStamp), &[LIST]),
        (Stub::Flaw(F::ListWrongDigest), &[LIST]),
        (Stub::Flaw(F::ListIgnoresTheChannel), &[LIST]),
        (Stub::Flaw(F::ListIgnoresThePrefix), &[LIST]),
        (Stub::Flaw(F::NewestIsEmpty), &[PUBLIC_READ, NEWEST]),
        (Stub::Flaw(F::ListIsEmpty), &[LIST]),
        (Stub::Flaw(F::TheMarkIsDropped), &[COMMIT_READABLE, STAMP]),
        (
            Stub::Flaw(F::LaterMembersHaveAWrongDigest),
            &[COMMIT_READABLE, ECHO],
        ),
        (
            Stub::Flaw(F::LaterMembersLoseTheirStamp),
            &[COMMIT_READABLE],
        ),
        (Stub::Flaw(F::LaterMembersLoseTheirMark), &[COMMIT_READABLE]),
        (Stub::Flaw(F::VersionYankReachesEveryVersion), &[YANK]),
        (Stub::More(More::CommitReplacesStaged), &[COMMIT_CARRIES]),
        (
            Stub::More(More::CommitUnstaged),
            &[COMMIT_CARRIES, VANISHED],
        ),
        (Stub::More(More::CommitPartial), &[DUPLICATE_SET]),
        (Stub::More(More::StagePartial), &[DUPLICATE_SET]),
        (Stub::More(More::DupStageDeduped), &[NAMED_TWICE]),
        (Stub::More(More::DupCommitDeduped), &[NAMED_TWICE]),
        (Stub::More(More::DupObserveSetAccepted), &[NAMED_TWICE]),
        (
            Stub::More(More::FetchWithWriterGrantSeesStaged),
            &[WRITER_READS],
        ),
        (
            Stub::More(More::NewestWithWriterGrantIncludesStaged),
            &[WRITER_READS],
        ),
        (
            Stub::More(More::ListWithWriterGrantIncludesStaged),
            &[WRITER_READS],
        ),
        (Stub::More(More::NonStagingRaceOverwrites), &[NOT_STAGING]),
        (
            Stub::More(More::NonStagingCommitOverYanked),
            &[YANKED_COMMIT, NOT_STAGING],
        ),
        (
            Stub::More(More::NonStagingTransientIsPermanent),
            &[NOT_STAGING],
        ),
        (
            Stub::More(More::NonStagingSetCommitIsTorn),
            &[SET_COMMIT, NOT_STAGING],
        ),
        (Stub::More(More::YankAbsentOk), &[YANK]),
        (Stub::More(More::StageAfterAbortIsDuplicate), &[ABORT]),
        (Stub::Flaw(F::VanishedStateIsCommitted), &[VANISHED]),
        (
            Stub::Flaw(F::RetryWhileBecomingReadableIsPermanent),
            &[BECOMING],
        ),
        (
            Stub::Flaw(F::UnrepresentableObserveIsAbsent),
            &[UNREPRESENTABLE],
        ),
        (
            Stub::Flaw(F::UnrepresentableCommitIsPermanent),
            &[UNREPRESENTABLE],
        ),
        (
            Stub::Hook(Hook::UnrepresentableIsTheBase),
            &[UNREPRESENTABLE, DECLARES],
        ),
        (Stub::Odd(Odd::DeclaresSealsOnly), &[SEALED]),
        (Stub::Odd(Odd::SealedIsDuplicate), &[SEALED]),
        (Stub::Odd(Odd::SealedIsDuplicateOnOneCall), &[SEALED]),
        (
            Stub::Odd(Odd::SealsUndeclared),
            &[OBSERVE_ABSENT, ABORT, YANK, NEWEST, COMMIT_CARRIES],
        ),
        (Stub::Flaw(F::BurnedReadsAsAbsent), &[BURNED]),
        (Stub::Flaw(F::BurnedCommitIsCommitted), &[BURNED]),
        (Stub::Flaw(F::EchoesTheDeclaredDigest), &[ECHO]),
        (Stub::Flaw(F::GrantLimitIsZero), &[DECLARES]),
        (
            Stub::Flaw(F::CommitUnitSaysSet),
            &[
                COMMIT_READABLE,
                SET_COMMIT,
                YANK,
                LIST,
                NEWEST,
                DUPLICATE_SET,
                ECHO,
                DECLARES,
            ],
        ),
        (Stub::Flaw(F::CommitUnitSaysCoordinate), &[SET_COMMIT]),
        (Stub::Flaw(F::VersionYankIsPerCoordinate), &[YANK]),
        (Stub::Flaw(F::VersionYankIgnoresTheGrant), &[YANK]),
        (Stub::Hook(Hook::CoordinateDropsThePrefix), &[DECLARES]),
        (
            Stub::Hook(Hook::PayloadIgnoresTheSeed),
            &[COMMIT_CARRIES, DECLARES],
        ),
        (
            Stub::Hook(Hook::PayloadIsEmpty),
            &[COMMIT_CARRIES, DECLARES],
        ),
        (Stub::Hook(Hook::SiblingDiffersInTwoFields), &[DECLARES]),
        (
            Stub::Hook(Hook::SiblingIsTheBase),
            &[
                OBSERVE_ABSENT,
                ABORT,
                YANK,
                LIST,
                PHASE,
                NEWEST,
                COMMIT_CARRIES,
                DECLARES,
            ],
        ),
        (Stub::Odd(Odd::NoShapes), &[DECLARES]),
        // A set refused for a grant that names too few of its coordinates is a set refused for
        // one coordinate: the others are not applied.
        (
            Stub::More(More::CommitPartialOnRefusal),
            &[SET_COMMIT, PHASE, UNREPRESENTABLE],
        ),
        (
            Stub::More(More::StagePartialOnRefusal),
            &[PHASE, UNREPRESENTABLE],
        ),
        (Stub::More(More::YankWidensToTheCommitSet), &[YANK]),
        (Stub::More(More::YankByNamePrefix), &[YANK]),
        (Stub::More(More::ObserveByNamePrefix), &[OBSERVE_ABSENT]),
        (Stub::More(More::FetchByNamePrefix), &[OBSERVE_ABSENT]),
        (
            Stub::More(More::CommitPublishesOtherStagedBytes),
            &[COMMIT_CARRIES],
        ),
        (
            Stub::More(More::DuplicateRestamps),
            &[SECOND_COMMIT, BECOMING],
        ),
        (Stub::More(More::AbortDropsUnnamedCommitted), &[ABORT]),
        (
            Stub::More(More::ObserveOfOthersReadsTheLaggingList),
            &[LIST],
        ),
        (Stub::More(More::EchoesOnAMultiMemberSet), &[ECHO]),
        (Stub::More(More::EchoesOnAYankedCoordinate), &[ECHO]),
        (Stub::More(More::StageDupOverwritesHeld), &[DUPLICATE_SET]),
        (Stub::More(More::StageDupYanksHeld), &[DUPLICATE_SET]),
        (Stub::More(More::CommitDupYanksHeld), &[DUPLICATE_SET]),
        (
            Stub::More(More::CommitPublishesSiblingStagedBytes),
            &[COMMIT_CARRIES],
        ),
        (Stub::More(More::AbortWithdrawsCommittedSibling), &[ABORT]),
        (
            Stub::More(More::StagedRaceOverwrites),
            &[RACE, SECOND_COMMIT],
        ),
        (Stub::More(More::NonStagingDupCommitDeduped), &[NAMED_TWICE]),
        (Stub::More(More::VanishedCommitIsRateLimited), &[VANISHED]),
        (
            Stub::More(More::RetryWhileBecomingReplacesTheStamp),
            &[BECOMING],
        ),
        (
            Stub::More(More::RetryWhileBecomingReplacesTheMark),
            &[BECOMING],
        ),
        (
            Stub::More(More::FetchOfAStrangerFindsNothing),
            &[PUBLIC_READ, READ_GRANT, ECHO],
        ),
        (Stub::Odd(Odd::OnlyAPlatformSibling), &[DECLARES]),
        (Stub::Siblingless(More::Conforming), &[DECLARES]),
        (
            Stub::Siblingless(More::CommitPublishesSiblingStagedBytes),
            &[DECLARES],
        ),
        (
            Stub::Siblingless(More::AbortWithdrawsCommittedSibling),
            &[DECLARES],
        ),
        (
            Stub::Siblingless(More::YankSiblings(Field::Version)),
            &[DECLARES],
        ),
        (
            Stub::Siblingless(More::StageSiblingTaken(Field::Version)),
            &[DECLARES],
        ),
        (
            Stub::Siblingless(More::ObserveStagedSibling(Field::Version)),
            &[DECLARES],
        ),
    ];
    for (call, bad) in every_bad_grant_taken_by_a_call() {
        table.push((
            Stub::Flaw(F::BadGrantTakenBy(bad, call)),
            match bad {
                Bad::Foreign => &[FOREIGN],
                Bad::Expired => &[EXPIRED],
                Bad::Uncovered => &[PHASE],
            },
        ));
    }
    table.push((Stub::More(More::AbortTakesCommitGrant), &[PHASE]));
    let lax = field_blind()
        .into_iter()
        .chain(on_shapes())
        .chain(coverage_of_two())
        .chain(read_grant_writes())
        .chain(writers_see_staged())
        .chain(read_grants_see_staged())
        .chain([More::ReadSeesOthersStaged])
        .chain(partial_coverage())
        .chain(writer_grants_on_reads())
        .chain(strangers_let_in())
        .chain(others_staged_seen())
        .chain(others_committed_blind());
    for stub in lax {
        table.push((Stub::More(stub), held_by_the_lax_stub(stub)));
    }
    for field in Field::ALL {
        // `newest` is asked of a name that has a sibling in the channel, the version and the
        // platform, and of none that differs in its type.
        let by_newest = field != Field::Type;
        let staged: &[&str] = if by_newest {
            &[OBSERVE_ABSENT, ABORT, YANK, NEWEST, COMMIT_CARRIES]
        } else {
            &[OBSERVE_ABSENT, ABORT, YANK, COMMIT_CARRIES]
        };
        let committed: &[&str] = if by_newest {
            &[OBSERVE_ABSENT, YANK, NEWEST, COMMIT_CARRIES]
        } else {
            &[OBSERVE_ABSENT, YANK, COMMIT_CARRIES]
        };
        table.extend([
            (Stub::More(More::YankSiblings(field)), &[YANK][..]),
            (Stub::More(More::AbortSiblings(field)), &[ABORT][..]),
            (Stub::More(More::StageSiblingTaken(field)), staged),
            (Stub::More(More::CommitSiblingTaken(field)), committed),
            (
                Stub::More(More::ObserveStagedSibling(field)),
                &[OBSERVE_ABSENT, ABORT][..],
            ),
        ]);
    }
    table
}

/// The stub of each scenario fails that scenario, and the kit has one for every scenario of it,
/// in its order.
#[test]
fn each_scenario_fails_on_its_nonconforming_stub() {
    let stubs = own_stubs();
    let named: Vec<&str> = stubs.iter().map(|(scenario, _)| *scenario).collect();
    assert_eq!(
        named,
        names(&publisher_scenarios::<FakePublisherRig>()),
        "the table has a stub for each scenario, in order"
    );
    for (scenario, stub) in stubs {
        let failed = run(stub).failed();
        assert!(
            failed.contains(&scenario),
            "the stub {stub:?} must fail {scenario}, and it failed {failed:?}"
        );
    }
}

/// Every stub fails exactly the scenarios it names: no scenario fails for a promise it does not
/// hold, and none lets a stub of the promise pass.
#[test]
fn each_further_publisher_stub_fails_the_scenarios_that_hold_it() {
    let table = every_stub();
    assert_eq!(table.len(), 419, "the number of stubs of the table");
    further_stubs_fail_exactly(
        "publisher",
        &names(&publisher_scenarios::<FakePublisherRig>()),
        &table,
        run,
    );
}

/// The scenarios about a private channel or one addressed by group, which a rig of public
/// channels lacks.
const NEEDS_A_PRIVATE_OR_GROUPED_CHANNEL: [&str; 6] = [
    GROUP_THEN_INDEX,
    ANOTHER_GROUP,
    FAILED_READ,
    DECLARED_SET,
    EMPTY_SET,
    READ_GRANT,
];

/// The scenarios that need a channel that stages, a private or a grouped one, or a registry that
/// seals a version, which a rig of one public channel that does not stage lacks.
const NEEDS_MORE_THAN_ONE_PUBLIC_CALL: [&str; 20] = [
    STAGE_OBSERVE,
    STRANGER_STAGED,
    STAGE_GRANT,
    COMMIT_GRANT,
    STAGE_PRESENT,
    ABORT,
    GROUP_THEN_INDEX,
    ANOTHER_GROUP,
    FAILED_READ,
    DECLARED_SET,
    EMPTY_SET,
    READ_GRANT,
    RACE,
    TRANSIENT,
    DUP_OTHER,
    OWN_STAGED,
    COMMIT_CARRIES,
    WRITER_READS,
    VANISHED,
    SEALED,
];

/// A rig that builds only a public channel whose commit carries the bytes skips exactly the
/// scenarios about staged state, about private and grouped channels and about a sealed version,
/// says why, passes every other, and a skip is not counted as a pass.
#[test]
fn a_rig_of_one_call_to_a_commit_skips_exactly_the_scenarios_it_cannot_run() {
    let report = run_publisher_kit(&FakePublisherRig::restricted());
    assert_eq!(report.skipped(), NEEDS_MORE_THAN_ONE_PUBLIC_CALL.to_vec());
    assert_eq!(report.failed(), Vec::<&str>::new());
    let ran: Vec<&str> = names(&publisher_scenarios::<FakePublisherRig>())
        .into_iter()
        .filter(|n| !NEEDS_MORE_THAN_ONE_PUBLIC_CALL.contains(n))
        .collect();
    assert_eq!(report.passed(), ran);
    assert_eq!(
        ran.len() + NEEDS_MORE_THAN_ONE_PUBLIC_CALL.len(),
        report.names().len()
    );
    let reasons = report.skip_reasons();
    assert!(reasons[0].starts_with(&format!("{STAGE_OBSERVE}: ")));
    assert!(reasons[0].contains("that stages"), "{reasons:?}");
    report.assert_conforms();
}

/// The stub of each scenario a rig of one call still runs, in the kit's order.
fn the_stub_of_each_scenario_a_rig_of_one_call_runs() -> Vec<(&'static str, Stub)> {
    use Flaw as F;
    vec![
        (OBSERVE_ABSENT, Stub::Flaw(F::AbsentIsAnError)),
        (COMMIT_READABLE, Stub::Flaw(F::PresentDigestIsWrong)),
        (COMMIT_OVER, Stub::Flaw(F::CommitOverOtherBytesOverwrites)),
        (SET_COMMIT, Stub::More(More::NonStagingSetCommitIsTorn)),
        (NOT_STAGE_BYTES, Stub::Flaw(F::StagesLies)),
        (NO_DIGEST, Stub::Flaw(F::OthersHaveNoDigest)),
        (YANK, Stub::Flaw(F::YankedIsNotFetchable)),
        (YANKED_COMMIT, Stub::More(More::NonStagingCommitOverYanked)),
        (LOST, Stub::Flaw(F::ObserveAfterALostAnswerIsAbsent)),
        (LIST, Stub::Flaw(F::ObserveReadsTheLaggingList)),
        (PUBLIC_READ, Stub::Flaw(F::AnonymousFetchIsDenied)),
        (FOREIGN, Stub::Flaw(F::AForeignGrantIsAccepted)),
        (PHASE, Stub::Flaw(F::TheWrongPhaseIsAccepted)),
        (EXPIRED, Stub::Flaw(F::AnExpiredGrantIsAccepted)),
        (NEWEST, Stub::Flaw(F::NewestDropsYanked)),
        (STAMP, Stub::Flaw(F::TheStampIsDropped)),
        (OTHERS_OBSERVE, Stub::Flaw(F::GrantedObserveIsBlindToOthers)),
        (
            SECOND_COMMIT,
            Stub::Flaw(F::SecondCommitOfTheSameBytesIsCommitted),
        ),
        (DUPLICATE_SET, Stub::More(More::CommitPartial)),
        (NAMED_TWICE, Stub::More(More::NonStagingDupCommitDeduped)),
        (NOT_STAGING, Stub::More(More::NonStagingRaceOverwrites)),
        (
            BECOMING,
            Stub::Flaw(F::RetryWhileBecomingReadableIsPermanent),
        ),
        (
            UNREPRESENTABLE,
            Stub::Flaw(F::UnrepresentableObserveIsAbsent),
        ),
        (BURNED, Stub::Flaw(F::BurnedReadsAsAbsent)),
        (ECHO, Stub::Flaw(F::EchoesTheDeclaredDigest)),
        (DECLARES, Stub::Flaw(F::GrantLimitIsZero)),
    ]
}

/// A rig that declares both kinds of channel holds a registry to every promise on the channel
/// that does not stage as well as on the one that stages: each stub of a scenario that a rig of
/// one call still runs, put behind the channel that does not stage alone, is failed by that
/// scenario, though the channel that stages is the conforming fake.
#[test]
fn a_stub_behind_the_channel_that_does_not_stage_alone_fails_its_scenario_on_a_rig_of_both() {
    for (scenario, stub) in the_stub_of_each_scenario_a_rig_of_one_call_runs() {
        let failed = run_behind_the_channel_that_does_not_stage(stub).failed();
        assert!(
            failed.contains(&scenario),
            "the stub {stub:?} behind the channel that does not stage must fail {scenario}, and it failed {failed:?}"
        );
    }
}

/// Each scenario the rig of one call still runs fails a stub of the promise it holds: a scenario
/// that adapts to a channel that does not stage still checks something there.
#[test]
fn each_scenario_a_rig_of_one_call_still_runs_fails_its_nonconforming_stub() {
    let rows = the_stub_of_each_scenario_a_rig_of_one_call_runs();
    let ran: Vec<&str> = rows.iter().map(|(scenario, _)| *scenario).collect();
    let restricted = run_publisher_kit(&FakePublisherRig::restricted());
    assert_eq!(
        ran,
        restricted.passed(),
        "a stub for each scenario the rig runs, in order"
    );
    for (scenario, stub) in rows {
        let failed = run_on(FakePublisherRig::restricted(), stub).failed();
        assert!(
            failed.contains(&scenario),
            "the stub {stub:?} must fail {scenario} on a rig of one call, and it failed {failed:?}"
        );
    }
}

/// A rig that lacks a thing a scenario needs skips that scenario and passes the rest: a registry
/// whose commit may replace what is there, one that commits each coordinate on its own, one that
/// cannot burn a number, one with no coordinate it cannot hold, one that stages and nothing else.
#[test]
fn a_rig_that_lacks_what_a_scenario_needs_skips_it_and_passes_the_rest() {
    let staging_only = FakePublisherRig::conforming().with_shapes(vec![ChannelShape::staging()]);
    let rigs = [
        (FakePublisherRig::unconditional(), vec![COMMIT_OVER, SEALED]),
        (
            FakePublisherRig::per_coordinate(),
            vec![DUPLICATE_SET, SEALED],
        ),
        (FakePublisherRig::unable_to_burn(), vec![SEALED, BURNED]),
        (
            FakePublisherRig::holding_every_coordinate(),
            vec![UNREPRESENTABLE, SEALED],
        ),
        (
            staging_only,
            [
                &[COMMIT_OVER, NOT_STAGE_BYTES][..],
                &NEEDS_A_PRIVATE_OR_GROUPED_CHANNEL,
                &[NOT_STAGING, SEALED],
            ]
            .concat(),
        ),
    ];
    for (rig, skipped) in rigs {
        let report = run_publisher_kit(&rig);
        assert_eq!(report.failed(), Vec::<&str>::new());
        assert_eq!(report.skipped(), skipped);
    }
}

/// A rig that builds public channels only skips exactly the scenarios about private and grouped
/// channels and about a sealed version, says why, and passes every other: a stranger's reads and
/// the grants of a public channel are held, and a skip is never a pass.
#[test]
fn a_rig_of_public_channels_skips_the_scenarios_about_private_and_grouped_ones() {
    let report = run_publisher_kit(&rig_of_public_channels());
    report.assert_conforms();
    let skipped = [&NEEDS_A_PRIVATE_OR_GROUPED_CHANNEL[..], &[SEALED]].concat();
    let in_order: Vec<&str> = names(&publisher_scenarios::<FakePublisherRig>())
        .into_iter()
        .filter(|n| skipped.contains(n))
        .collect();
    assert_eq!(report.skipped(), in_order);
    assert_eq!(report.passed().len(), report.names().len() - skipped.len());
    let reasons = report.skip_reasons();
    assert!(
        reasons
            .iter()
            .any(|r| r.starts_with(&format!("{READ_GRANT}: ")) && r.contains("no private channel")),
        "{reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|r| r.starts_with(&format!("{GROUP_THEN_INDEX}: ")) && r.contains("by group")),
        "{reasons:?}"
    );
    // What it still runs is held: the stub of each such scenario that is wrong about a public
    // channel fails it.
    for (scenario, stub) in own_stubs() {
        if skipped.contains(&scenario) {
            continue;
        }
        if matches!(stub, Stub::Odd(_)) {
            continue;
        }
        let failed = run_on(rig_of_public_channels(), stub).failed();
        assert!(
            failed.contains(&scenario),
            "the stub {stub:?} must fail {scenario} on a rig of public channels, and it failed {failed:?}"
        );
    }
}

/// A rig of the public channels, one that stages and one that does not.
fn rig_of_public_channels() -> FakePublisherRig {
    FakePublisherRig::conforming().with_shapes(vec![
        ChannelShape::staging(),
        ChannelShape::without_staging(),
    ])
}

/// A rig that declares no shape fails the kit, whatever else it passes, and a scenario run alone
/// on it fails with a reason and does not panic.
#[test]
fn a_rig_that_declares_no_shape_fails_the_kit() {
    let report = run(Stub::Odd(Odd::NoShapes));
    assert_eq!(report.failed(), vec![DECLARES]);
    assert_eq!(report.passed(), Vec::<&str>::new());
    let rig = FakePublisherRig::conforming().with_shapes(Vec::new());
    let scenarios = publisher_scenarios::<FakePublisherRig>();
    let alone = scenarios.iter().find(|s| s.name == OBSERVE_ABSENT);
    let why = alone.map(|s| s.run_on(&rig));
    assert!(
        matches!(&why, Some(Err(text)) if text.contains("a channel to run on")),
        "{why:?}"
    );
}
