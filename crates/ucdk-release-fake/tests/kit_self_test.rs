// SPDX-License-Identifier: Apache-2.0
//! The self-test of the kits: for every scenario of every kit, a stub of the seam that breaks
//! that one promise and no other, and the proof that the kit then fails that scenario and only
//! that scenario. A scenario no stub can fail is a scenario that checks nothing.

mod rigs;

use rigs::build_system::{FakeBuildRig, Flaw as Build};
use rigs::grant_source::{FakeGrantRig, Flaw as Grants};
use rigs::output_files::{FakeFilesRig, Flaw as Files};
use rigs::process_runner::{FakeProcessRig, Flaw as Process};
use rigs::publisher::{Bad, Call, FakePublisherRig, Flaw as Registry};
use rigs::store::{FakeStoreRig, Flaw as Store};
use ucdk_release_proto::v1::BuildSystemKind;
use ucdk_release_seams::kit::{
    Report, build_system_scenarios, grant_source_scenarios, output_files_scenarios,
    process_runner_scenarios, publisher_scenarios, run_build_system_kit, run_grant_source_kit,
    run_output_files_kit, run_process_runner_kit, run_publisher_kit, run_store_kit,
    store_scenarios,
};

/// Holds that the table names every scenario of the kit, in its order, and that each stub fails
/// its scenario alone.
fn each_fails_alone<F: Copy>(
    kit: &str,
    scenarios: &[&'static str],
    table: &[(&'static str, F)],
    run: impl Fn(F) -> Report,
) {
    let named: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        named, scenarios,
        "{kit}: the table has a stub for each scenario, in order"
    );
    for (name, flaw) in table {
        let report = run(*flaw);
        assert_eq!(
            report.failed(),
            vec![*name],
            "{kit}: the stub for {name} must fail that scenario and no other: {:?}",
            report.outcomes
        );
    }
}

/// Holds that each further stub fails the scenarios it names, which are in the kit's order, and
/// no other. These are the wrong implementations that a first version of a scenario let pass.
fn further_stubs_fail_exactly<F: Copy + std::fmt::Debug>(
    kit: &str,
    scenarios: &[&'static str],
    table: &[(F, &[&'static str])],
    run: impl Fn(F) -> Report,
) {
    let mut wrong = Vec::new();
    for (flaw, named) in table {
        let in_order: Vec<&str> = scenarios
            .iter()
            .copied()
            .filter(|s| named.contains(s))
            .collect();
        assert_eq!(
            in_order.as_slice(),
            *named,
            "{kit}: {flaw:?} names scenarios the kit has, in the kit's order"
        );
        let failed = run(*flaw).failed();
        if failed != *named {
            wrong.push(format!("{flaw:?}: failed {failed:?}, named {named:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{kit}: each stub must fail exactly the scenarios it names:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn each_further_output_files_stub_fails_the_scenarios_that_hold_it() {
    further_output_files();
}

#[test]
fn each_further_store_stub_fails_the_scenarios_that_hold_it() {
    further_store();
}

#[test]
fn each_further_grant_source_stub_fails_the_scenarios_that_hold_it() {
    further_grant_source();
}

#[test]
fn each_further_process_runner_stub_fails_the_scenarios_that_hold_it() {
    further_process_runner();
}

#[test]
fn each_further_build_system_stub_fails_the_scenarios_that_hold_it() {
    further_build_system();
}

#[test]
fn each_further_publisher_stub_fails_the_scenarios_that_hold_it() {
    further_publisher();
}

fn further_store() {
    const READ_BACK: &str = "an_entry_reads_back_what_was_written";
    const ESCAPES: &str = "paths_that_escape_an_entry_are_refused";
    further_stubs_fail_exactly(
        "store",
        &names(&store_scenarios::<FakeStoreRig>()),
        &[
            (Store::EmptyFileIsDropped, &[READ_BACK]),
            (Store::GetIsLossy, &[READ_BACK]),
            (Store::GetTruncatesAt4k, &[READ_BACK]),
            (Store::GetOfADirectoryIsEmpty, &[READ_BACK]),
            (Store::WriterAcceptsATrailingSlash, &[ESCAPES]),
            (Store::WriterAcceptsADotPart, &[ESCAPES]),
            (Store::WriterAcceptsControlCharacters, &[ESCAPES]),
            (Store::GetResolvesClimbs, &[ESCAPES]),
            (Store::DiscardWipesEveryEntry, &[]),
            (Store::DiscardOfAMissingEntryIsAnError, &[]),
            (Store::FilesMatchedByKeyPrefix, &[]),
            (Store::PathsAreCaseFolded, &[]),
            (Store::GetTakesATrailingSlash, &[]),
            (Store::EmptyEntryIsNoEntry, &[]),
        ],
        |flaw| run_store_kit(&FakeStoreRig::flawed(flaw)),
    );
}

fn further_grant_source() {
    const COVERS: &str = "a_grant_covers_exactly_the_coordinates_asked";
    const DIFFERING: &str = "a_differing_claim_is_refused_before_any_exchange";
    const EXCEPTIONAL: &str = "an_exceptional_run_presents_the_exceptional_subject";
    const LIMIT: &str = "more_coordinates_than_the_limit_are_refused_before_the_exchange";
    further_stubs_fail_exactly(
        "grant source",
        &names(&grant_source_scenarios::<FakeGrantRig>()),
        &[
            (Grants::CommitGrantHasTheStageScope, &[COVERS]),
            (Grants::CommitGrantDropsACoordinate, &[COVERS]),
            (Grants::SkipsClaimsOutsideStage, &[DIFFERING, EXCEPTIONAL]),
            (Grants::ClaimsMatchedBySubstring, &[DIFFERING]),
            (Grants::NoPinnedClaimIsIssued, &[DIFFERING]),
            (Grants::ExceptionalIgnoresEnvironment, &[DIFFERING]),
            (Grants::ExceptionalIgnoresAudienceAndPinned, &[DIFFERING]),
            (Grants::LimitIsCheckedForStageOnly, &[LIMIT]),
            (Grants::IgnoresTheDeclaredScopesAndLocation, &[]),
            (Grants::ClaimsMatchedIgnoringCase, &[]),
        ],
        |flaw| run_grant_source_kit(&FakeGrantRig::flawed(flaw)),
    );
}

fn further_output_files() {
    const READ_THROUGH: &str = "an_entry_of_a_listed_directory_is_read_through_its_joined_path";
    const KINDS: &str = "a_listing_names_the_kind_of_every_entry";
    const REFUSED: &str = "listing_a_file_and_reading_a_directory_or_a_symlink_are_refused";
    const STAT: &str = "stat_tells_a_file_a_directory_a_symlink_and_nothing_apart";
    const DESCENDED: &str = "a_listed_symlink_is_never_descended";
    further_stubs_fail_exactly(
        "output files",
        &names(&output_files_scenarios::<FakeFilesRig>()),
        &[
            (Files::LossyRead, &[READ_THROUGH]),
            (Files::EmptyFileIsMissing, &[READ_THROUGH]),
            (Files::ReadTruncatesAt4k, &[READ_THROUGH]),
            (Files::DotfilesHidden, &[READ_THROUGH]),
            (Files::DeepEntriesOmitted, &[READ_THROUGH]),
            (Files::LinkToDirListedAsDirectory, &[KINDS]),
            (Files::DanglingLinkOmitted, &[KINDS]),
            (Files::ListOfSymlinkToDirFollows, &[REFUSED]),
            (Files::TrailingSlashFollowsRead, &[REFUSED]),
            (Files::TrailingSlashFollowsStat, &[STAT]),
            (Files::ReadRefusesDotDotNames, &[READ_THROUGH]),
            (Files::FollowsNestedLinks, &[DESCENDED]),
            (Files::StatBelowAFileIsAnError, &[]),
            (Files::ListOfAnEmptyDirectoryIsPermanent, &[]),
            (Files::ReadTruncatesAt64k, &[]),
        ],
        |flaw| run_output_files_kit(&FakeFilesRig::flawed(flaw)),
    );
}

fn further_process_runner() {
    further_stubs_fail_exactly(
        "process runner",
        &names(&process_runner_scenarios::<FakeProcessRig>()),
        &[
            (
                Process::CwdIgnored,
                &["argv_and_environment_are_exactly_what_was_given"],
            ),
            (
                Process::NoPathFallsBackToOwn,
                &["a_name_found_nowhere_on_the_childs_path_is_unstartable"],
            ),
            (Process::AStopPoisonsTheRunner, &[]),
            (Process::EnvironmentLeaksIntoTheNextRun, &[]),
            (Process::LastPathEntryWins, &[]),
        ],
        |flaw| run_process_runner_kit(&FakeProcessRig::flawed(flaw)),
    );
}

fn further_build_system() {
    const SAME_BYTES: &str = "same_command_same_bytes";
    const COPY_OUT: &str = "copy_out_writes_only_into_the_entry_it_is_given";
    const FLAGS: &str = "base_flags_come_before_target_flags_in_order";
    const REPLACES: &str = "a_target_environment_value_replaces_the_base_value_of_the_same_key";
    const INHERITS: &str = "nothing_is_inherited_from_the_process_environment";
    const PLACEHOLDERS: &str = "placeholders_are_substituted_in_flag_and_environment_values";
    const STAMP: &str = "the_stamp_reaches_a_build_only_through_placeholders";
    const ONE_OUTPUT: &str = "a_build_lists_exactly_one_output";
    const SYMLINK: &str = "a_symlink_in_an_output_is_refused";
    let scenarios = names(&build_system_scenarios::<FakeBuildRig>());
    for kind in [BuildSystemKind::Buck2, BuildSystemKind::Bazel] {
        further_stubs_fail_exactly(
            "build system",
            &scenarios,
            &[
                (Build::CopyOutWritesGarbage, &[SAME_BYTES]),
                (Build::CopyOutWritesAnEmptyFile, &[SAME_BYTES]),
                (Build::CopyOutWritesAnExtraFile, &[COPY_OUT]),
                (Build::TestVerbTargetFlagsFirst, &[FLAGS]),
                (Build::TestVerbBaseEnvironmentWins, &[REPLACES]),
                (Build::TestVerbInheritsTheEnvironment, &[INHERITS]),
                (
                    Build::TestVerbDropsTheEnvironment,
                    &[REPLACES, INHERITS, PLACEHOLDERS],
                ),
                (
                    Build::BaseFlagsRepeatedAfterTheOwnedFlag,
                    &[FLAGS, "ucdk_owned_flags_come_after_the_authors"],
                ),
                (Build::DirectoryIsListedAsAFile, &[ONE_OUTPUT]),
                (Build::DirectoryOutputIsRefused, &[ONE_OUTPUT, COPY_OUT]),
                (Build::NestedSymlinkIsAccepted, &[SYMLINK]),
                (Build::CopyOutOfADirectoryEscapes, &[COPY_OUT]),
                (Build::CopyOutOfADirectoryWritesGarbage, &[]),
                (Build::CopyOutOfADirectoryCarriesTheClock, &[]),
            ],
            |flaw| run_build_system_kit(&FakeBuildRig::flawed(kind, flaw)),
        );
    }
    // Only Bazel has the two verbs that locate an output.
    further_stubs_fail_exactly(
        "build system (Bazel verbs)",
        &scenarios,
        &[
            (Build::QueryVerbInheritsTheEnvironment, &[INHERITS]),
            (Build::InfoVerbLeaksTheStamp, &[STAMP]),
        ],
        |flaw| run_build_system_kit(&FakeBuildRig::flawed(BuildSystemKind::Bazel, flaw)),
    );
}

fn further_publisher() {
    const PRIVATE: &str =
        "a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit";
    const STRANGER: &str = "public_read_needs_no_grant";
    const NEWEST: &str = "newest_lists_every_held_coordinate_of_the_name";
    const ABORT: &str = "abort_gives_staged_state_back_and_never_touches_a_committed_coordinate";
    const PHASE: &str = "a_grant_of_the_wrong_phase_is_denied_and_nothing_happens";
    const ABSENT: &str = "observe_absent_is_not_an_error";
    const LIST: &str = "list_may_lag_but_observe_is_exact";
    const DECLARED: &str =
        "observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set";
    further_stubs_fail_exactly(
        "publisher",
        &names(&publisher_scenarios::<FakePublisherRig>()),
        &[
            (Registry::StrangerObservesPrivate, &[PRIVATE]),
            (Registry::StrangerFetchesPrivate, &[PRIVATE]),
            (Registry::StrangerNewestPrivate, &[PRIVATE]),
            (Registry::StrangerListsPrivate, &[PRIVATE]),
            (Registry::StrangerFetchesStaged, &[STRANGER]),
            (Registry::ListIncludesStaged, &[STRANGER]),
            (Registry::NewestIncludesStaged, &[STRANGER]),
            (Registry::NewestMatchesByPrefix, &[NEWEST]),
            (Registry::NewestIgnoresTheChannel, &[NEWEST]),
            (Registry::AbortGivesBackAllStaged, &[ABORT]),
            (Registry::YankIgnoresThePhase, &[PHASE]),
            (Registry::StageIgnoresCoverage, &[PHASE]),
            (Registry::CommitIgnoresCoverage, &[PHASE]),
            (Registry::StrangerMayStage, &[PHASE]),
            (Registry::StrangerMayCommit, &[PHASE]),
            (Registry::StrangerMayAbort, &[PHASE]),
            (Registry::StrangerMayYank, &[PHASE]),
            (Registry::PrivateTakesAForeignReadGrant, &[PRIVATE]),
            (Registry::PrivateTakesAnExpiredReadGrant, &[PRIVATE]),
            (Registry::PrivateTakesAnUncoveredReadGrant, &[PRIVATE]),
            (Registry::ObserveIgnoresVersion, &[ABSENT]),
            (Registry::ObserveIgnoresPlatform, &[ABSENT]),
            (Registry::ObserveIgnoresChannel, &[ABSENT]),
            (Registry::FetchIgnoresVersion, &[ABSENT]),
            (Registry::FetchIgnoresPlatform, &[ABSENT]),
            (Registry::FetchIgnoresChannel, &[ABSENT]),
            (Registry::NewestDropsTheStamp, &[NEWEST]),
            (Registry::NewestWrongDigest, &[NEWEST]),
            (Registry::NewestDropsTheExceptionalMark, &[NEWEST]),
            (Registry::ListDropsTheStamp, &[LIST]),
            (Registry::ListWrongDigest, &[LIST]),
            (Registry::ListIgnoresTheChannel, &[LIST]),
            (Registry::ListIgnoresThePrefix, &[LIST]),
            (Registry::ObserveDropsAForeignSet, &[DECLARED]),
            (Registry::StrangerObservesPrivateWithASet, &[]),
            (Registry::BadGrantTakenBy(Bad::Foreign, Call::Commit), &[]),
            (Registry::BadGrantTakenBy(Bad::Foreign, Call::Abort), &[]),
            (Registry::BadGrantTakenBy(Bad::Foreign, Call::Yank), &[]),
            (Registry::BadGrantTakenBy(Bad::Expired, Call::Commit), &[]),
            (Registry::BadGrantTakenBy(Bad::Expired, Call::Abort), &[]),
            (Registry::BadGrantTakenBy(Bad::Expired, Call::Yank), &[]),
            (Registry::BadGrantTakenBy(Bad::Uncovered, Call::Abort), &[]),
            (Registry::BadGrantTakenBy(Bad::Uncovered, Call::Yank), &[]),
            (Registry::OnlyTheLastCoordinateIsCovered, &[]),
            (Registry::SecondCommitOfTheSameBytesIsCommitted, &[]),
            (Registry::StageOverOwnStagedSameBytesIsStaged, &[]),
            (Registry::StageOverOwnStagedOverwrites, &[]),
        ],
        |flaw| run_publisher_kit(&FakePublisherRig::flawed(flaw)),
    );
}

fn names<R>(scenarios: &[ucdk_release_seams::kit::Scenario<R>]) -> Vec<&'static str> {
    scenarios.iter().map(|s| s.name).collect()
}

#[test]
fn each_scenario_fails_on_its_nonconforming_stub() {
    output_files();
    store();
    grant_source();
    process_runner();
    build_system();
    publisher();
}

fn output_files() {
    each_fails_alone(
        "output files",
        &names(&output_files_scenarios::<FakeFilesRig>()),
        &[
            ("a_listing_is_sorted_relative_paths", Files::WalkOrder),
            (
                "a_listing_names_the_kind_of_every_entry",
                Files::EveryEntryIsAFile,
            ),
            ("a_listed_symlink_is_never_descended", Files::FollowsLinks),
            (
                "stat_tells_a_file_a_directory_a_symlink_and_nothing_apart",
                Files::StatFollows,
            ),
            (
                "a_missing_path_is_none_from_stat_and_permanent_from_list_and_read",
                Files::ListOfMissingIsEmpty,
            ),
            (
                "listing_a_file_and_reading_a_directory_or_a_symlink_are_refused",
                Files::ReadFollowsLink,
            ),
            (
                "a_single_file_output_is_read_without_a_rel",
                Files::NeedsListedDirAtTheRoot,
            ),
            (
                "a_report_file_is_read_directly",
                Files::NeedsListedDirBelowTheRoot,
            ),
            (
                "reading_a_directory_is_refused_even_when_it_is_the_located_output",
                Files::ReadOfAStattedDirectoryIsEmpty,
            ),
            (
                "a_path_that_is_relative_or_climbs_is_refused",
                Files::ReadResolvesClimbs,
            ),
            (
                "an_entry_of_a_listed_directory_is_read_through_its_joined_path",
                Files::NestedEntryIsUnreadable,
            ),
            (
                "a_read_returns_the_file_as_it_is_at_the_call",
                Files::CachesReads,
            ),
            ("no_call_changes_the_tree", Files::ReadTouches),
        ],
        |flaw| run_output_files_kit(&FakeFilesRig::flawed(flaw)),
    );
}

fn store() {
    each_fails_alone(
        "store",
        &names(&store_scenarios::<FakeStoreRig>()),
        &[
            (
                "an_entry_reads_back_what_was_written",
                Store::GetLosesNestedPaths,
            ),
            ("an_entry_is_listed_exactly", Store::FilesAreNotSorted),
            (
                "a_second_begin_on_one_key_is_refused",
                Store::SecondBeginResets,
            ),
            (
                "keys_of_two_scopes_are_listed_apart",
                Store::KeysCollapseByMemberName,
            ),
            (
                "paths_that_escape_an_entry_are_refused",
                Store::WriterAcceptsClimbingPaths,
            ),
            (
                "two_entries_do_not_touch_each_other",
                Store::GetIgnoresTheKey,
            ),
            (
                "discard_removes_every_path_of_an_entry",
                Store::DiscardLeavesTheFirstPath,
            ),
            (
                "a_missing_entry_lists_as_empty",
                Store::MissingEntryIsAnError,
            ),
        ],
        |flaw| run_store_kit(&FakeStoreRig::flawed(flaw)),
    );
}

fn grant_source() {
    each_fails_alone(
        "grant source",
        &names(&grant_source_scenarios::<FakeGrantRig>()),
        &[
            (
                "an_anonymous_identity_needs_no_exchange",
                Grants::AnonymousGetsAGrant,
            ),
            (
                "a_grant_covers_exactly_the_coordinates_asked",
                Grants::DropsTheLastCoordinate,
            ),
            (
                "a_differing_claim_is_refused_before_any_exchange",
                Grants::IgnoresPinnedClaims,
            ),
            (
                "an_exceptional_run_presents_the_exceptional_subject",
                Grants::IgnoresExceptional,
            ),
            (
                "a_used_identity_token_is_refused_the_second_time",
                Grants::UsedIsDenied,
            ),
            (
                "an_exchange_after_expiry_gives_a_new_grant",
                Grants::ReissuesTheExpiredGrant,
            ),
            (
                "a_missing_scope_is_denied",
                Grants::CommitWithoutScopeIsIssued,
            ),
            (
                "a_read_grant_carries_the_read_scope_and_covers_exactly_the_coordinates_asked",
                Grants::ReadIsIssuedAsStage,
            ),
            (
                "an_unreachable_issuer_is_unreachable_and_spends_nothing",
                Grants::UnreachableSpendsTheToken,
            ),
            (
                "an_expiry_is_reported_as_the_issuer_stated_it",
                Grants::CorrectsTheSkew,
            ),
            (
                "more_coordinates_than_the_limit_are_refused_before_the_exchange",
                Grants::AllowsOneOverTheLimit,
            ),
            (
                "a_grant_is_never_printed_cloned_or_logged",
                Grants::LogsTheSecret,
            ),
        ],
        |flaw| run_grant_source_kit(&FakeGrantRig::flawed(flaw)),
    );
}

fn process_runner() {
    each_fails_alone(
        "process runner",
        &names(&process_runner_scenarios::<FakeProcessRig>()),
        &[
            (
                "a_bare_name_is_resolved_on_the_childs_path_and_never_the_runners",
                Process::ResolvesOnItsOwnPath,
            ),
            (
                "a_name_found_nowhere_on_the_childs_path_is_unstartable",
                Process::BareNameNotFoundIsPermanent,
            ),
            (
                "argv_and_environment_are_exactly_what_was_given",
                Process::InheritsTheEnvironment,
            ),
            (
                "no_shell_interprets_an_argument",
                Process::ShellExpandsAnArgument,
            ),
            (
                "exit_code_and_signal_are_distinguished",
                Process::SignalIsAnExitCode,
            ),
            (
                "unstartable_is_not_a_nonzero_exit",
                Process::UnstartableIsAnExit,
            ),
            (
                "output_is_bounded_and_its_tail_is_kept",
                Process::KeepsTheHead,
            ),
            (
                "the_runner_beats_on_progress_and_stops_the_child_when_told",
                Process::IgnoresStop,
            ),
            (
                "a_stopped_child_and_its_process_group_are_gone",
                Process::LeavesTheGroupRunning,
            ),
        ],
        |flaw| run_process_runner_kit(&FakeProcessRig::flawed(flaw)),
    );
}

fn build_system() {
    for kind in [BuildSystemKind::Buck2, BuildSystemKind::Bazel] {
        each_fails_alone(
            "build system",
            &names(&build_system_scenarios::<FakeBuildRig>()),
            &[
                ("same_command_same_bytes", Build::CallCounterInTheBytes),
                (
                    "base_flags_come_before_target_flags_in_order",
                    Build::TargetFlagsFirst,
                ),
                (
                    "a_target_environment_value_replaces_the_base_value_of_the_same_key",
                    Build::BaseEnvironmentWins,
                ),
                (
                    "nothing_is_inherited_from_the_process_environment",
                    Build::InheritsTheEnvironment,
                ),
                (
                    "placeholders_are_substituted_in_flag_and_environment_values",
                    Build::LeavesTheRevisionPlaceholder,
                ),
                (
                    "the_stamp_reaches_a_build_only_through_placeholders",
                    Build::StampLeaksIntoTheCommand,
                ),
                (
                    "ucdk_owned_flags_come_after_the_authors",
                    Build::OwnedFlagFirst,
                ),
                (
                    "a_build_lists_exactly_one_output",
                    Build::ListsTheOutputTwice,
                ),
                ("no_output_is_build_output", Build::NoOutputIsAnEmptyListing),
                (
                    "two_outputs_are_build_output",
                    Build::TwoOutputsAnswersTheFirst,
                ),
                (
                    "a_symlink_in_an_output_is_refused",
                    Build::SymlinkIsAccepted,
                ),
                (
                    "a_failed_build_is_permanent_with_the_tail_of_its_output",
                    Build::FailureHasNoTail,
                ),
                ("unstartable_is_not_failed", Build::UnstartableIsPermanent),
                ("nothing_run_specific_in_the_bytes", Build::ClockInTheBytes),
                (
                    "copy_out_writes_only_into_the_entry_it_is_given",
                    Build::CopyOutEscapes,
                ),
                (
                    "a_test_target_that_runs_nothing_is_not_a_pass",
                    Build::NothingRanPasses,
                ),
                ("a_failing_test_target_is_not_a_pass", Build::FailureIsAPass),
                (
                    "a_test_report_that_cannot_be_read_is_not_a_pass",
                    Build::UnparseableReportPasses,
                ),
                (
                    "the_build_system_beats_on_progress_and_stops_when_told",
                    Build::StopIsSwallowed,
                ),
            ],
            |flaw| run_build_system_kit(&FakeBuildRig::flawed(kind, flaw)),
        );
    }
}

fn publisher() {
    each_fails_alone(
        "publisher",
        &names(&publisher_scenarios::<FakePublisherRig>()),
        &[
            ("observe_absent_is_not_an_error", Registry::AbsentIsAnError),
            (
                "stage_then_observe_returns_staged_with_the_same_digest",
                Registry::StagedDigestIsWrong,
            ),
            (
                "a_staged_coordinate_is_not_readable_by_a_stranger",
                Registry::StrangerSeesStaged,
            ),
            (
                "a_stage_grant_observes_the_staged_state_it_made",
                Registry::StageGrantIsBlindToStaged,
            ),
            (
                "a_commit_grant_observes_it_too",
                Registry::CommitGrantIsBlindToStaged,
            ),
            (
                "commit_makes_the_staged_coordinates_readable_with_the_same_digest",
                Registry::PresentDigestIsWrong,
            ),
            (
                "stage_of_present_same_is_duplicate_and_changes_nothing",
                Registry::StageOverTheSameBytesIsStaged,
            ),
            (
                "commit_over_present_different_leaves_old_bytes",
                Registry::CommitOverOtherBytesOverwrites,
            ),
            (
                "abort_gives_staged_state_back_and_never_touches_a_committed_coordinate",
                Registry::AbortDoesNothing,
            ),
            (
                "a_set_commit_makes_every_coordinate_readable_or_none",
                Registry::SetCommitIsTorn,
            ),
            (
                "a_channel_that_does_not_stage_commits_with_the_bytes",
                Registry::StagesLies,
            ),
            ("no_digest_exposed_is_refused", Registry::OthersHaveNoDigest),
            ("yank_keeps_bytes_fetchable", Registry::YankedIsNotFetchable),
            (
                "yanked_coordinate_cannot_be_committed_again",
                Registry::CommitOverYankedCommits,
            ),
            (
                "lost_answer_is_resolvable_by_observe",
                Registry::ObserveAfterALostAnswerIsAbsent,
            ),
            (
                "list_may_lag_but_observe_is_exact",
                Registry::ObserveReadsTheLaggingList,
            ),
            (
                "public_read_needs_no_grant",
                Registry::AnonymousFetchIsDenied,
            ),
            (
                "a_grant_for_another_repository_sends_nothing",
                Registry::AForeignGrantIsAccepted,
            ),
            (
                "a_grant_of_the_wrong_phase_is_denied_and_nothing_happens",
                Registry::TheWrongPhaseIsAccepted,
            ),
            (
                "an_expired_grant_is_refused_and_nothing_happens",
                Registry::AnExpiredGrantIsAccepted,
            ),
            (
                "newest_lists_every_held_coordinate_of_the_name",
                Registry::NewestDropsYanked,
            ),
            (
                "observe_returns_the_stamp_it_was_given",
                Registry::TheStampIsDropped,
            ),
            (
                "a_granted_observe_sees_a_coordinate_committed_by_another_writer",
                Registry::GrantedObserveIsBlindToOthers,
            ),
            (
                "a_private_observe_reads_the_group_then_the_index",
                Registry::IndexIsReadBeforeTheGroup,
            ),
            (
                "a_coordinate_committed_in_another_group_reads_as_present_never_absent",
                Registry::GroupedObserveIsBlindToOthers,
            ),
            (
                "a_failed_group_read_or_index_read_fails_the_observe_and_is_not_absent",
                Registry::FailedGroupReadIsAbsent,
            ),
            (
                "observe_with_the_declared_set_finds_staged_state_addressed_by_the_group_of_that_set",
                Registry::TheDeclaredSetIsDropped,
            ),
            (
                "observe_with_an_empty_set_cannot_see_staged_state_and_still_sees_committed_coordinates",
                Registry::AnEmptySetSeesStagedState,
            ),
            (
                "a_read_grant_observes_committed_coordinates_and_can_neither_stage_nor_commit",
                Registry::AReadGrantMayWrite,
            ),
            (
                "commit_is_atomic_when_the_coordinate_appears_between_observe_and_commit",
                Registry::ARacedCommitOverwrites,
            ),
            (
                "transient_after_commit_is_observed_before_retry",
                Registry::TransientAfterCommitIsPermanent,
            ),
            (
                "duplicate_over_other_bytes_leaves_the_old_bytes",
                Registry::StageOverOtherBytesIsStaged,
            ),
        ],
        |flaw| run_publisher_kit(&FakePublisherRig::flawed(flaw)),
    );
}
