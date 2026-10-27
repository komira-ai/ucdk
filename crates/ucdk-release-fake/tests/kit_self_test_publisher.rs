// SPDX-License-Identifier: Apache-2.0
//! The self-test of the publisher kit, first part: for every scenario a stub of the registry that
//! breaks the promise it holds and is failed by it, the further stubs that a first version of a
//! scenario let pass, the declarations a rig can get wrong, and a rig that can build one kind of
//! channel only, which must skip exactly the scenarios that need another and fail every stub of the
//! ones it still runs.

mod rigs;

use rigs::publisher::{FakePublisherRig, Flaw, Hook};
use rigs::publisher_more::{Field, More, MoreRig};
use rigs::{further_stubs_fail_exactly, names};
use ucdk_release_seams::kit::{ChannelShape, Report, publisher_scenarios, run_publisher_kit};

const OBSERVE_ABSENT: &str = "observe_absent_is_not_an_error";
const STAGE_OBSERVE: &str = "stage_then_observe_returns_staged_with_the_same_digest";
const STRANGER_STAGED: &str = "a_staged_coordinate_is_not_readable_by_a_stranger";
const STAGE_GRANT: &str = "a_stage_grant_observes_the_staged_state_it_made";
const COMMIT_GRANT: &str = "a_commit_grant_observes_it_too";
const COMMIT_READABLE: &str = "commit_makes_the_staged_coordinates_readable_with_the_same_digest";
const STAGE_PRESENT: &str = "stage_of_present_same_is_duplicate_and_changes_nothing";
const COMMIT_OVER: &str = "commit_over_present_different_leaves_old_bytes";
const ABORT: &str = "abort_gives_staged_state_back_and_never_touches_a_committed_coordinate";
const SET_COMMIT: &str = "a_set_commit_makes_every_coordinate_readable_or_none";
const NOT_STAGE_BYTES: &str = "a_channel_that_does_not_stage_commits_with_the_bytes";
const NO_DIGEST: &str = "no_digest_exposed_is_refused";
const YANK: &str = "yank_keeps_bytes_fetchable";
const YANKED_COMMIT: &str = "yanked_coordinate_cannot_be_committed_again";
const LOST: &str = "lost_answer_is_resolvable_by_observe";
const LIST: &str = "list_may_lag_but_observe_is_exact";
const SECOND_COMMIT: &str = "a_second_commit_of_the_same_bytes_is_duplicate";
const OWN_STAGED: &str = "a_stage_over_ones_own_staged_coordinate_is_duplicate";
const COMMIT_CARRIES: &str = "a_commit_carries_exactly_what_was_staged";
const DUPLICATE_SET: &str = "a_duplicate_answer_leaves_every_coordinate_of_the_set_as_it_was";
const NAMED_TWICE: &str = "a_set_that_names_a_coordinate_twice_is_refused";
const WRITER_READS: &str = "a_stage_or_commit_grant_reads_no_staged_state_but_through_observe";
const NOT_STAGING: &str = "a_channel_that_does_not_stage_creates_a_coordinate_once";
const VANISHED: &str =
    "a_commit_of_staged_state_that_vanished_is_not_committed_and_a_restage_works";
const BECOMING: &str = "a_retry_of_a_commit_that_is_becoming_readable_is_committed_or_duplicate_and_never_a_second_write";
const UNREPRESENTABLE: &str =
    "a_coordinate_the_channel_cannot_hold_is_refused_before_anything_is_sent";
const SEALED: &str = "a_sealed_version_refuses_a_later_platform_and_observes_it_absent";
const BURNED: &str = "a_burned_coordinate_is_neither_absent_nor_present_nor_committable";
const ECHO: &str = "a_registry_that_echoes_the_declared_digest_fails_readback";
const DECLARES: &str = "the_declarations_of_a_rig_hold_of_what_it_builds";

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
}

#[derive(Debug, Clone, Copy)]
enum Stub {
    Flaw(Flaw),
    More(More),
    Hook(Hook),
    Odd(Odd),
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
    }
}

fn run(stub: Stub) -> Report {
    run_on(FakePublisherRig::conforming(), stub)
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

/// Every stub with every scenario it fails, in the kit's order: the stub of each scenario, and
/// the further stubs.
fn every_stub() -> Vec<(Stub, &'static [&'static str])> {
    use Flaw as F;
    let mut table: Vec<(Stub, &'static [&'static str])> = vec![
        (Stub::Flaw(F::AbsentIsAnError), &[OBSERVE_ABSENT]),
        (Stub::Flaw(F::StagedDigestIsWrong), &[STAGE_OBSERVE, ECHO]),
        (Stub::Flaw(F::StrangerSeesStaged), &[STRANGER_STAGED]),
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
                COMMIT_OVER,
                NOT_STAGE_BYTES,
                SECOND_COMMIT,
                DUPLICATE_SET,
                NOT_STAGING,
                DECLARES,
            ],
        ),
        (Stub::Flaw(F::OthersHaveNoDigest), &[NO_DIGEST]),
        (Stub::Flaw(F::YankedIsNotFetchable), &[YANK]),
        (Stub::Flaw(F::CommitOverYankedCommits), &[YANKED_COMMIT]),
        (Stub::Flaw(F::ObserveAfterALostAnswerIsAbsent), &[LOST]),
        (Stub::Flaw(F::ObserveReadsTheLaggingList), &[LIST]),
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
        (Stub::Flaw(F::FetchIgnoresVersion), &[OBSERVE_ABSENT]),
        (Stub::Flaw(F::FetchIgnoresPlatform), &[OBSERVE_ABSENT]),
        (Stub::Flaw(F::FetchIgnoresChannel), &[OBSERVE_ABSENT]),
        (Stub::Flaw(F::ListDropsTheStamp), &[LIST]),
        (Stub::Flaw(F::ListWrongDigest), &[LIST]),
        (Stub::Flaw(F::ListIgnoresTheChannel), &[LIST]),
        (Stub::Flaw(F::ListIgnoresThePrefix), &[LIST]),
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
        (Stub::More(More::NonStagingCommitOverYanked), &[NOT_STAGING]),
        (
            Stub::More(More::NonStagingTransientIsPermanent),
            &[NOT_STAGING],
        ),
        (Stub::More(More::NonStagingSetCommitIsTorn), &[NOT_STAGING]),
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
        (Stub::Odd(Odd::SealsUndeclared), &[OBSERVE_ABSENT, YANK]),
        (Stub::Flaw(F::BurnedReadsAsAbsent), &[BURNED]),
        (Stub::Flaw(F::BurnedCommitIsCommitted), &[BURNED]),
        (Stub::Flaw(F::EchoesTheDeclaredDigest), &[ECHO]),
        (Stub::Flaw(F::GrantLimitIsZero), &[DECLARES]),
        (
            Stub::Flaw(F::CommitUnitSaysSet),
            &[COMMIT_READABLE, SET_COMMIT, LIST, DUPLICATE_SET, DECLARES],
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
            &[OBSERVE_ABSENT, ABORT, YANK, LIST, DECLARES],
        ),
        (Stub::Odd(Odd::NoShapes), &[DECLARES]),
    ];
    for field in Field::ALL {
        table.extend([
            (Stub::More(More::YankSiblings(field)), &[YANK][..]),
            (Stub::More(More::AbortSiblings(field)), &[ABORT][..]),
            (
                Stub::More(More::StageSiblingTaken(field)),
                &[OBSERVE_ABSENT, ABORT, YANK][..],
            ),
            (
                Stub::More(More::CommitSiblingTaken(field)),
                &[OBSERVE_ABSENT, YANK][..],
            ),
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
    further_stubs_fail_exactly(
        "publisher",
        &names(&publisher_scenarios::<FakePublisherRig>()),
        &table,
        run,
    );
}

/// The scenarios that need a channel that stages, and the one that needs a registry that seals a
/// version, which a rig of one public channel that does not stage lacks.
const NEEDS_STAGING_OR_SEALING: [&str; 12] = [
    STAGE_OBSERVE,
    STRANGER_STAGED,
    STAGE_GRANT,
    COMMIT_GRANT,
    STAGE_PRESENT,
    ABORT,
    OWN_STAGED,
    COMMIT_CARRIES,
    NAMED_TWICE,
    WRITER_READS,
    VANISHED,
    SEALED,
];

/// A rig that builds only a public channel whose commit carries the bytes skips exactly the
/// scenarios about staged state and about a sealed version, says why, passes every other, and a
/// skip is not counted as a pass.
#[test]
fn a_rig_of_one_call_to_a_commit_skips_exactly_the_scenarios_it_cannot_run() {
    let report = run_publisher_kit(&FakePublisherRig::restricted());
    assert_eq!(report.skipped(), NEEDS_STAGING_OR_SEALING.to_vec());
    assert_eq!(report.failed(), Vec::<&str>::new());
    let ran: Vec<&str> = names(&publisher_scenarios::<FakePublisherRig>())
        .into_iter()
        .filter(|n| !NEEDS_STAGING_OR_SEALING.contains(n))
        .collect();
    assert_eq!(report.passed(), ran);
    assert_eq!(
        ran.len() + NEEDS_STAGING_OR_SEALING.len(),
        report.names().len()
    );
    let reasons = report.skip_reasons();
    assert!(reasons[0].starts_with(&format!("{STAGE_OBSERVE}: ")));
    assert!(reasons[0].contains("that stages"), "{reasons:?}");
    report.assert_conforms();
}

/// Each scenario the rig of one call still runs fails a stub of the promise it holds: a scenario
/// that adapts to a channel that does not stage still checks something there.
#[test]
fn each_scenario_a_rig_of_one_call_still_runs_fails_its_nonconforming_stub() {
    use Flaw as F;
    let rows: Vec<(&str, Stub)> = vec![
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
        (
            SECOND_COMMIT,
            Stub::Flaw(F::SecondCommitOfTheSameBytesIsCommitted),
        ),
        (DUPLICATE_SET, Stub::More(More::CommitPartial)),
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
    ];
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
            vec![COMMIT_OVER, NOT_STAGE_BYTES, NOT_STAGING, SEALED],
        ),
    ];
    for (rig, skipped) in rigs {
        let report = run_publisher_kit(&rig);
        assert_eq!(report.failed(), Vec::<&str>::new());
        assert_eq!(report.skipped(), skipped);
    }
}

/// A rig that builds only private channels has no channel this part of the kit can run on: it
/// skips every scenario that needs one, and passes only the one about what it declares. A kit
/// that skipped everything is not a kit that passed.
#[test]
fn a_rig_with_no_public_channel_skips_what_runs_on_one_and_passes_nothing_else() {
    let private = ChannelShape {
        stages: true,
        private: true,
        group_addressed: true,
    };
    let report = run_publisher_kit(&FakePublisherRig::conforming().with_shapes(vec![private]));
    assert_eq!(report.failed(), Vec::<&str>::new());
    assert_eq!(report.passed(), vec![DECLARES]);
    assert_eq!(report.skipped().len(), report.names().len() - 1);
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
        matches!(&why, Some(Err(text)) if text.contains("a public channel to run on")),
        "{why:?}"
    );
}
