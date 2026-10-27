// SPDX-License-Identifier: Apache-2.0
//! What a stub of the publisher gets wrong: one promise of a registry, broken.

use ucdk_release_seams::kit::ChannelShape;

use super::publisher_lax::Defect;

/// The channels a stub is behind: the channels of every other shape are the conforming fake, so
/// that a scenario that runs on one kind of channel only is held to fail the stub there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behind {
    /// The channels that do not stage, public or private.
    Unstaged,
    /// A public channel addressed by group.
    PublicGrouped,
    /// A private channel addressed by group.
    PrivateGrouped,
    /// A private channel that stages and is addressed by coordinate.
    PrivatePlain,
    /// A private channel that does not stage.
    PrivateUnstaged,
}

impl Behind {
    /// Every kind of channel that is not the public one addressed by coordinate.
    pub const ALL: [Self; 5] = [
        Self::Unstaged,
        Self::PublicGrouped,
        Self::PrivateGrouped,
        Self::PrivatePlain,
        Self::PrivateUnstaged,
    ];

    /// The kinds of channel that stage.
    pub const STAGING: [Self; 3] = [
        Self::PublicGrouped,
        Self::PrivateGrouped,
        Self::PrivatePlain,
    ];

    /// The kinds of channel that are private or addressed by group, which stage or do not.
    pub const NOT_PUBLIC_PLAIN: [Self; 4] = [
        Self::PublicGrouped,
        Self::PrivateGrouped,
        Self::PrivatePlain,
        Self::PrivateUnstaged,
    ];

    /// Whether a channel of `shape` is of this kind.
    pub fn holds(self, shape: &ChannelShape) -> bool {
        match self {
            Self::Unstaged => !shape.stages,
            Self::PublicGrouped => !shape.private && shape.group_addressed,
            Self::PrivateGrouped => shape.private && shape.group_addressed,
            Self::PrivatePlain => shape.private && shape.stages && !shape.group_addressed,
            Self::PrivateUnstaged => shape.private && !shape.stages,
        }
    }
}

/// A wrong declaration of a rig: it says of the coordinates, bytes and siblings it supplies a
/// thing that is not true, and the registry behind it is the conforming fake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hook {
    /// A label maps to a name that does not start with it.
    CoordinateDropsThePrefix,
    /// Two seeds give one member.
    PayloadIgnoresTheSeed,
    /// A member has no bytes.
    PayloadIsEmpty,
    /// A sibling differs from its base in two fields.
    SiblingDiffersInTwoFields,
    /// A sibling is the base itself.
    SiblingIsTheBase,
    /// The coordinate the channel cannot hold is the base of the scenarios.
    UnrepresentableIsTheBase,
}

/// How a grant is bad: the way a registry that is lax about it takes it for a good one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bad {
    /// It is for another repository.
    Foreign,
    /// It has expired.
    Expired,
    /// It is of the right phase and does not name the coordinates of the call.
    Uncovered,
}

/// A call of a registry that writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    Stage,
    Commit,
    Abort,
    Yank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// A coordinate nobody wrote is an error to observe.
    AbsentIsAnError,
    /// A staged coordinate is observed with a wrong digest.
    StagedDigestIsWrong,
    /// A stranger sees a staged coordinate.
    StrangerSeesStaged,
    /// A stage grant cannot see what it staged.
    StageGrantIsBlindToStaged,
    /// A commit grant cannot see staged state.
    CommitGrantIsBlindToStaged,
    /// A committed coordinate is observed with a wrong digest.
    PresentDigestIsWrong,
    /// A stage over the same bytes is answered `Staged`.
    StageOverTheSameBytesIsStaged,
    /// A commit over a coordinate that was not staged and holds other bytes replaces them.
    CommitOverOtherBytesOverwrites,
    /// An abort does nothing.
    AbortDoesNothing,
    /// A commit of a set that fails leaves the first coordinate readable, on a channel that
    /// stages.
    SetCommitIsTorn,
    /// A channel that does not stage says it does.
    StagesLies,
    /// A coordinate of another writer is observed without its digest.
    OthersHaveNoDigest,
    /// A yanked coordinate cannot be fetched.
    YankedIsNotFetchable,
    /// A commit over a yanked coordinate answers `Committed`, on a channel that stages.
    CommitOverYankedCommits,
    /// After an answer was lost, an observe answers `Absent`.
    ObserveAfterALostAnswerIsAbsent,
    /// While the listing lags, an observe reads the listing.
    ObserveReadsTheLaggingList,
    /// An abort by a stage grant gives back every staged coordinate, not only those it names.
    AbortGivesBackAllStaged,
    /// An `observe` of a coordinate nobody holds answers what a held coordinate of the same name
    /// and another version has.
    ObserveIgnoresVersion,
    /// The same, for a coordinate of another platform.
    ObserveIgnoresPlatform,
    /// The same, for a coordinate of another channel.
    ObserveIgnoresChannel,
    /// A `fetch` of a coordinate nobody holds answers the bytes of a held coordinate of the same
    /// name and another version.
    FetchIgnoresVersion,
    /// The same, for a coordinate of another platform.
    FetchIgnoresPlatform,
    /// The same, for a coordinate of another channel.
    FetchIgnoresChannel,
    /// `list` answers each coordinate without its stamp.
    ListDropsTheStamp,
    /// `list` answers each coordinate with a wrong digest.
    ListWrongDigest,
    /// `list` answers the coordinates of the prefix in every channel.
    ListIgnoresTheChannel,
    /// `list` answers every coordinate of the channel, whatever the prefix.
    ListIgnoresThePrefix,
    /// A second commit of bytes that are already committed answers `Committed`.
    SecondCommitOfTheSameBytesIsCommitted,
    /// A stage over a coordinate the caller staged, with the same bytes, answers `Staged`.
    StageOverOwnStagedSameBytesIsStaged,
    /// A stage over a coordinate the caller staged, with other bytes, replaces them and answers
    /// `Staged`.
    StageOverOwnStagedOverwrites,
    /// A commit of staged state that vanished answers `Committed`.
    VanishedStateIsCommitted,
    /// A commit repeated while the first is becoming readable is `Permanent`.
    RetryWhileBecomingReadableIsPermanent,
    /// An `observe` of a coordinate the channel cannot hold answers `Absent`.
    UnrepresentableObserveIsAbsent,
    /// A `commit` of a coordinate the channel cannot hold is `Permanent`, not `Refused`.
    UnrepresentableCommitIsPermanent,
    /// A later platform of a sealed version is answered `Duplicate`.
    SealedVersionIsDuplicate,
    /// A burned coordinate is observed as `Absent`.
    BurnedReadsAsAbsent,
    /// A commit over a burned coordinate answers `Committed`.
    BurnedCommitIsCommitted,
    /// An `observe` reports the digest the writer declared when there is one.
    EchoesTheDeclaredDigest,
    /// The channel says no grant may cover any coordinate.
    GrantLimitIsZero,
    /// A channel that commits each coordinate on its own says it commits a set.
    CommitUnitSaysSet,
    /// A channel that commits a set says it commits each coordinate on its own.
    CommitUnitSaysCoordinate,
    /// A channel that yanks a version says so and yanks only the coordinate named.
    VersionYankIsPerCoordinate,
    /// A channel that yanks a version says so and yanks every platform, whatever the grant
    /// covers.
    VersionYankIgnoresTheGrant,
    /// A read without a grant of a public channel is denied, whatever it reads but an observe.
    AnonymousFetchIsDenied,
    /// A grant for another repository is taken for this one.
    AForeignGrantIsAccepted,
    /// A grant of the other write phase is taken for the phase of the call.
    TheWrongPhaseIsAccepted,
    /// An expired grant is taken for a good one.
    AnExpiredGrantIsAccepted,
    /// `newest` leaves out a yanked coordinate.
    NewestDropsYanked,
    /// A committed coordinate is observed without its stamp.
    TheStampIsDropped,
    /// A grant on a plain channel does not see what another writer committed.
    GrantedObserveIsBlindToOthers,
    /// The group and the index are read in the other order.
    IndexIsReadBeforeTheGroup,
    /// A grant on a grouped channel does not see what another writer committed.
    GroupedObserveIsBlindToOthers,
    /// A failed group read is taken for no group.
    FailedGroupReadIsAbsent,
    /// The declared set is dropped from an observe of staged state.
    TheDeclaredSetIsDropped,
    /// An observe with no set sees staged state on a grouped channel.
    AnEmptySetSeesStagedState,
    /// A read grant may stage, commit, abort and yank.
    AReadGrantMayWrite,
    /// A commit that lost a race replaces the bytes that won it.
    ARacedCommitOverwrites,
    /// A failure after a commit that landed is `Permanent`, on a channel that stages.
    TransientAfterCommitIsPermanent,
    /// A stage over the bytes of another writer is answered `Staged`.
    StageOverOtherBytesIsStaged,
    /// A stranger's `observe` of a private channel is answered as a read grant's would be.
    StrangerObservesPrivate,
    /// A stranger's `fetch` of a private channel is answered as a read grant's would be.
    StrangerFetchesPrivate,
    /// A stranger's `newest` of a private channel is answered as a read grant's would be.
    StrangerNewestPrivate,
    /// A stranger's `list` of a private channel is answered as a read grant's would be.
    StrangerListsPrivate,
    /// A stranger fetches the bytes of a staged coordinate.
    StrangerFetchesStaged,
    /// `list` names staged coordinates.
    ListIncludesStaged,
    /// `newest` names staged coordinates.
    NewestIncludesStaged,
    /// `newest` answers every coordinate whose name starts with the name asked for.
    NewestMatchesByPrefix,
    /// `newest` answers the coordinates of the name in every channel.
    NewestIgnoresTheChannel,
    /// A yank is taken from a stage grant.
    YankIgnoresThePhase,
    /// A stage grant that does not cover every coordinate of the call is taken for one that does.
    StageIgnoresCoverage,
    /// A commit grant that does not cover every coordinate of the call is taken for one that
    /// does.
    CommitIgnoresCoverage,
    /// A stage with no grant is taken for a stage with a stage grant.
    StrangerMayStage,
    /// A commit with no grant is taken for a commit with a commit grant.
    StrangerMayCommit,
    /// An abort with no grant is taken for an abort with a stage grant.
    StrangerMayAbort,
    /// A yank with no grant is taken for a yank with a commit grant.
    StrangerMayYank,
    /// A private channel takes a read grant for another repository for its own, on every read.
    PrivateTakesAForeignReadGrant,
    /// A private channel takes an expired read grant for a good one, on every read.
    PrivateTakesAnExpiredReadGrant,
    /// A private channel takes a read grant that does not name the coordinate asked for as one
    /// that does, on `observe` and `fetch`.
    PrivateTakesAnUncoveredReadGrant,
    /// `newest` answers each coordinate without its stamp.
    NewestDropsTheStamp,
    /// `newest` answers each coordinate with a wrong digest.
    NewestWrongDigest,
    /// `newest` answers each coordinate as not exceptional.
    NewestDropsTheExceptionalMark,
    /// An `observe` told a set that does not hold the coordinate observes it as if told nothing.
    ObserveDropsAForeignSet,
    /// A stranger's `observe` of a private channel that is told a non-empty set is answered as a
    /// stage grant's would be.
    StrangerObservesPrivateWithASet,
    /// The call takes a grant that is bad in the way given for a good one, and every other call
    /// refuses it.
    BadGrantTakenBy(Bad, Call),
    /// A stage or a commit takes a grant for every coordinate of the call when it names the
    /// last of them.
    OnlyTheLastCoordinateIsCovered,
    /// `newest` answers nothing, whatever the channel holds.
    NewestIsEmpty,
    /// `list` answers nothing, whatever the channel holds.
    ListIsEmpty,
    /// An observe answers each committed coordinate as not exceptional.
    TheMarkIsDropped,
    /// An observe of a committed coordinate that was not the first of its call answers a wrong
    /// digest.
    LaterMembersHaveAWrongDigest,
    /// An observe of a committed coordinate that was not the first of its call answers no stamp.
    LaterMembersLoseTheirStamp,
    /// An observe of a committed coordinate that was not the first of its call answers it as not
    /// exceptional.
    LaterMembersLoseTheirMark,
    /// A channel that yanks a version says so and yanks the platforms of every version of the
    /// name when the grant covers the version it names.
    VersionYankReachesEveryVersion,
    /// A publisher that does not look at a grant that is bad in the way given: it sends the call
    /// with the secret of the grant, the registry refuses it, and the refusal is the answer.
    SendsRefused(Defect),
}
