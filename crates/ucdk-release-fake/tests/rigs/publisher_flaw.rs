// SPDX-License-Identifier: Apache-2.0
//! What a stub of the publisher gets wrong: one promise of a registry, broken.

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
}
