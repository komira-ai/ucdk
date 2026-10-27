// SPDX-License-Identifier: Apache-2.0
//! What a stub of the publisher gets wrong: one promise of a registry, broken.

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
    /// An abort by a stage grant gives back every staged coordinate, not only those it names.
    AbortGivesBackAllStaged,
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
    /// `newest` answers each coordinate without its stamp.
    NewestDropsTheStamp,
    /// `newest` answers each coordinate with a wrong digest.
    NewestWrongDigest,
    /// `newest` answers each coordinate as not exceptional.
    NewestDropsTheExceptionalMark,
    /// `list` answers each coordinate without its stamp.
    ListDropsTheStamp,
    /// `list` answers each coordinate with a wrong digest.
    ListWrongDigest,
    /// `list` answers the coordinates of the prefix in every channel.
    ListIgnoresTheChannel,
    /// `list` answers every coordinate of the channel, whatever the prefix.
    ListIgnoresThePrefix,
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
    /// A second commit of bytes that are already committed answers `Committed`.
    SecondCommitOfTheSameBytesIsCommitted,
    /// A stage over a coordinate the caller staged, with the same bytes, answers `Staged`.
    StageOverOwnStagedSameBytesIsStaged,
    /// A stage over a coordinate the caller staged, with other bytes, replaces them and answers
    /// `Staged`.
    StageOverOwnStagedOverwrites,
}
