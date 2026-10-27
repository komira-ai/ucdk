// SPDX-License-Identifier: Apache-2.0
//! The self-test of the kits: for every scenario of a kit, a stub of the seam that breaks
//! that one promise and no other, and the proof that the kit then fails that scenario and only
//! that scenario. A scenario no stub can fail is a scenario that checks nothing.

mod rigs;

use rigs::build_system::{FakeBuildRig, Flaw as Build};
use rigs::hostile::{HostileRig, Wrong};
use rigs::loose::{LooseRig, ShallowRig};
use rigs::output_files::{FakeFilesRig, Flaw as Files};
use rigs::process_runner::{FakeProcessRig, Flaw as Process};
use rigs::store::{FakeStoreRig, Flaw as Store};
use rigs::{each_fails_alone, further_stubs_fail_exactly, names};
use ucdk_release_proto::v1::BuildSystemKind;
use ucdk_release_seams::kit::{
    build_system_scenarios, output_files_scenarios, process_runner_scenarios, run_build_system_kit,
    run_output_files_kit, run_process_runner_kit, run_store_kit, store_scenarios,
};

#[test]
fn each_further_output_files_stub_fails_the_scenarios_that_hold_it() {
    further_output_files();
}

#[test]
fn each_further_store_stub_fails_the_scenarios_that_hold_it() {
    further_store();
}

#[test]
fn each_further_process_runner_stub_fails_the_scenarios_that_hold_it() {
    further_process_runner();
}

#[test]
fn each_further_build_system_stub_fails_the_scenarios_that_hold_it() {
    further_build_system();
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
    const RUN_SPECIFIC: &str = "nothing_run_specific_in_the_bytes";
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
                (
                    Build::DirectoryIsListedAsAFile,
                    &[SAME_BYTES, ONE_OUTPUT, COPY_OUT],
                ),
                (
                    Build::DirectoryOutputIsRefused,
                    &[SAME_BYTES, ONE_OUTPUT, RUN_SPECIFIC, COPY_OUT],
                ),
                (Build::NestedSymlinkIsAccepted, &[SYMLINK]),
                (Build::CopyOutOfADirectoryEscapes, &[COPY_OUT]),
                (Build::CopyOutOfADirectoryWritesGarbage, &[SAME_BYTES]),
                (Build::CopyOutOfADirectoryCarriesTheClock, &[RUN_SPECIFIC]),
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

fn further_process_runner() {
    const BARE: &str = "a_bare_name_is_resolved_on_the_childs_path_and_never_the_runners";
    const ARGV: &str = "argv_and_environment_are_exactly_what_was_given";
    const EXIT: &str = "exit_code_and_signal_are_distinguished";
    const OUTPUT: &str = "output_is_bounded_and_its_tail_is_kept";
    const NOWHERE: &str = "a_name_found_nowhere_on_the_childs_path_is_unstartable";
    const BEATS: &str = "the_runner_beats_on_progress_and_stops_the_child_when_told";
    const HELD: &str = "a_stopped_child_and_its_process_group_are_gone";
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
            (
                Process::AStopPoisonsTheRunner,
                &[
                    ARGV,
                    "the_runner_beats_on_progress_and_stops_the_child_when_told",
                ],
            ),
            (Process::EnvironmentLeaksIntoTheNextRun, &[BARE, ARGV]),
            (
                Process::LastPathEntryWins,
                &["a_bare_name_is_resolved_on_the_childs_path_and_never_the_runners"],
            ),
            (Process::CwdIsFirstRuns, &[BARE, NOWHERE, ARGV]),
            (Process::LimitIsFirstRuns, &[OUTPUT]),
            (Process::PathMissCached, &[BARE]),
            (Process::StderrTailAccumulates, &[OUTPUT]),
            (Process::EnvLeaksAfterAnUnstartableRun, &[ARGV]),
            (Process::EnvLeaksAfterAStop, &[ARGV]),
            (Process::SignalOtherThan9IsCode128Plus, &[EXIT]),
            (Process::HighCodeIsASignal, &[EXIT]),
            (Process::ZeroLimitMeansUnlimited, &[OUTPUT]),
            (Process::EnvValueWithEqualsTruncated, &[ARGV]),
            (Process::NonexistentPathEntryIsFatal, &[BARE]),
            (Process::EmptyEnvInherits, &[ARGV]),
            (Process::DefaultPathInEnv, &[ARGV]),
            (Process::CwdSearchedForBareNames, &[NOWHERE]),
            (Process::SlashNameSearchedOnPath, &[NOWHERE]),
            (Process::SlashNameRelativeToRunnersCwd, &[NOWHERE]),
            (Process::ArgsTrimmed, &[ARGV]),
            (Process::ArgBackslashEaten, &[ARGV]),
            (Process::ArgCutAtNewline, &[ARGV]),
            (Process::EnvKeyUppercased, &[BARE, ARGV]),
            (Process::EnvValueLowercased, &[ARGV]),
            (Process::ExitCode127IsUnstartable, &[EXIT]),
            (Process::NonzeroExitLosesStderr, &[EXIT]),
            (Process::NonzeroExitLosesStdout, &[EXIT]),
            (Process::StderrOfFailedRunLeaks, &[OUTPUT]),
            (Process::PathLimitedToTwo, &[BARE]),
            (Process::PathSplitsOnSpace, &[BARE]),
            (Process::UnknownSignalsAreCodes, &[EXIT]),
            (Process::BeatsCappedAtTwo, &[BEATS]),
            (Process::PathRemembered, &[BARE]),
            (Process::PathKeyCaseInsensitive, &[BARE]),
            (Process::RelativePathEntryUsesRunnerCwd, &[BARE]),
            (Process::EnvValueTrimmed, &[ARGV]),
            (Process::EnvValueCutAtNewline, &[ARGV]),
            (Process::EnvValueBackslashEaten, &[ARGV]),
            (Process::EnvValueNonAscii, &[ARGV]),
            (Process::Argv0ReSplitOnSpace, &[ARGV]),
            (Process::LimitKeptPerChunk, &[OUTPUT]),
            (Process::StdoutAtTheLimitIsDropped, &[OUTPUT]),
            (Process::NoBeatOnAnExitPoll, &[HELD]),
            (Process::KillsWithoutReaping, &[HELD]),
        ],
        |flaw| run_process_runner_kit(&FakeProcessRig::flawed(flaw)),
    );
}

fn further_store() {
    const READ_BACK: &str = "an_entry_reads_back_what_was_written";
    const ESCAPES: &str = "paths_that_escape_an_entry_are_refused";
    const LISTED: &str = "an_entry_is_listed_exactly";
    const SECOND_BEGIN: &str = "a_second_begin_on_one_key_is_refused";
    const KEYS: &str = "keys_of_two_scopes_are_listed_apart";
    const TWO: &str = "two_entries_do_not_touch_each_other";
    const DISCARD: &str = "discard_removes_every_path_of_an_entry";
    const MISSING: &str = "a_missing_entry_lists_as_empty";
    further_stubs_fail_exactly(
        "store",
        &names(&store_scenarios::<FakeStoreRig>()),
        &[
            (Store::EmptyFileIsDropped, &[READ_BACK]),
            (Store::GetIsLossy, &[READ_BACK]),
            (Store::GetTruncatesAt4k, &[READ_BACK]),
            (Store::GetOfADirectoryIsEmpty, &[READ_BACK, ESCAPES]),
            (Store::GetIgnoresTheKey, &[LISTED, TWO, DISCARD]),
            (
                Store::GetLosesNestedPaths,
                &[READ_BACK, LISTED, ESCAPES, TWO, DISCARD],
            ),
            (Store::WriterAcceptsATrailingSlash, &[ESCAPES]),
            (Store::WriterAcceptsADotPart, &[ESCAPES]),
            (Store::WriterAcceptsControlCharacters, &[ESCAPES]),
            (Store::GetResolvesClimbs, &[ESCAPES]),
            (Store::DiscardWipesEveryEntry, &[DISCARD]),
            (Store::DiscardOfAMissingEntryIsAnError, &[DISCARD]),
            (Store::FilesMatchedByKeyPrefix, &[KEYS, TWO, DISCARD]),
            (Store::PathsAreCaseFolded, &[LISTED, ESCAPES]),
            (Store::GetTakesATrailingSlash, &[ESCAPES]),
            (Store::EmptyEntryIsNoEntry, &[SECOND_BEGIN]),
            (Store::DeepPathsDropped, &[READ_BACK, ESCAPES, DISCARD]),
            (Store::DeepPathsNotListed, &[READ_BACK, ESCAPES]),
            (Store::DeepPathsGetNone, &[READ_BACK, ESCAPES, DISCARD]),
            (Store::DiscardDeepSurvives, &[DISCARD]),
            (Store::PutTruncatesAt64k, &[READ_BACK]),
            (Store::GetTruncatesAt64k, &[READ_BACK]),
            (Store::AppendOnRewrite, &[READ_BACK]),
            (Store::DiscardPrefixSiblings, &[DISCARD]),
            (Store::BeginRefusedOnExistingLongerKey, &[DISCARD]),
            (Store::KeysOmitsEmptyEntries, &[SECOND_BEGIN]),
            (Store::WriterPoisonedAfterRefusal, &[ESCAPES]),
            (Store::DotfilesHidden, &[LISTED]),
            (Store::TrailingSpaceTrimmed, &[LISTED, ESCAPES]),
            (Store::UnicodeFolded, &[LISTED, ESCAPES]),
            (Store::FilesCappedAt100, &[LISTED]),
            (Store::KeysCappedAt100, &[KEYS]),
            (Store::RewriteKeepsTheOldTail, &[READ_BACK]),
            (Store::WriterAcceptsAPathEndingInADotPart, &[ESCAPES]),
            (Store::SetKeyAliasesMemberSet, &[KEYS, TWO]),
            (Store::SetKeyRefused, &[KEYS, TWO, DISCARD]),
            (Store::PlatformIgnored, &[KEYS, TWO, DISCARD]),
            (Store::RevisionIgnored, &[KEYS, TWO, DISCARD]),
            (Store::KeysAreCaseFolded, &[KEYS, TWO]),
            (Store::KeysAreUnicodeFolded, &[KEYS, TWO]),
            (Store::KeysTrailingSpaceTrimmed, &[KEYS, TWO]),
            (Store::KeysNonAsciiAlike, &[KEYS, TWO]),
            (Store::DiscardSameMember, &[DISCARD]),
            (Store::DiscardMissingWipes, &[DISCARD]),
            (Store::DiscardMissingRemovesLast, &[DISCARD]),
            (Store::DiscardMissingCreatesEmpty, &[DISCARD]),
            (Store::RefusedPutOverwrites, &[ESCAPES]),
            (Store::GetPrefixFallback, &[TWO]),
            (Store::GetCaseFallback, &[LISTED]),
            (Store::GetUnicodeFallback, &[LISTED]),
            (Store::NestedRewriteKeepsTheOldTail, &[READ_BACK]),
            (Store::NestedEmptyIsDropped, &[READ_BACK]),
            (Store::EmptyThenNonEmptyIgnored, &[READ_BACK]),
            (Store::NestedEmptyRewriteIgnored, &[READ_BACK]),
            (Store::FilesNaturalSort, &[LISTED]),
            (Store::FilesUtf16Sort, &[LISTED]),
            (Store::StripsATrailingNewline, &[READ_BACK]),
            (Store::CrlfBecomesLf, &[READ_BACK]),
            (Store::StripsTrailingNuls, &[READ_BACK]),
            (Store::StripsTheBom, &[READ_BACK]),
            (Store::AllZeroStoredEmpty, &[READ_BACK]),
            (Store::LosesTheLastByteAtA4kMultiple, &[READ_BACK]),
            (Store::TruncatesAt1MiB, &[READ_BACK]),
            (Store::EqualLengthRewriteIgnored, &[READ_BACK]),
            (Store::DiscardSetKeyTakesTheMembers, &[DISCARD]),
            (Store::DiscardMemberTakesSetDocument, &[DISCARD]),
            (Store::GetSameMemberFallback, &[TWO]),
            (Store::FilesFallBackToTheSameMember, &[TWO]),
            (Store::WriterRefusesAnyDoubleDot, &[LISTED]),
            (Store::WriterRefusesAColon, &[LISTED]),
            (Store::WriterRefusesUrlCharacters, &[LISTED]),
            (Store::WriterRefusesPatternCharacters, &[LISTED]),
            (Store::WriterRefusesQuotesAndSeparators, &[LISTED]),
            (Store::WriterRefusesAnInnerSpace, &[LISTED]),
            (Store::WriterRewritesReservedCharacters, &[LISTED]),
            (Store::WriterEncodesASpace, &[LISTED]),
            (Store::WriterPercentDecodes, &[LISTED]),
            (Store::GetOfEmptyOrDotIsTheEntryRoot, &[ESCAPES]),
            (Store::GetCollapsesSlashes, &[ESCAPES]),
            (Store::GetTakesADoubledTrailingSlash, &[ESCAPES]),
            (Store::GetReadsABackslashAsASlash, &[ESCAPES]),
            (Store::GetTrimsTrailingControls, &[ESCAPES]),
            (Store::FilesCappedAt1000, &[LISTED]),
            (Store::KeysCappedAt1000, &[KEYS]),
            (Store::KeysCollapseAcrossGenerations, &[KEYS, TWO]),
            (Store::MissingEntryIsAnError, &[TWO, MISSING]),
            (Store::TruncatesAt16MiB, &[READ_BACK]),
            (Store::WriterRefusesReservedNames, &[LISTED]),
        ],
        |flaw| run_store_kit(&FakeStoreRig::flawed(flaw)),
    );
}

#[test]
fn a_store_that_lets_a_file_and_a_directory_share_a_name_fails_the_kit() {
    // The store is the fake in all else: only the scenario of refused paths holds it to the
    // promise that the paths of an entry form a tree.
    assert_eq!(
        run_store_kit(&LooseRig).failed(),
        ["paths_that_escape_an_entry_are_refused"]
    );
}

#[test]
fn a_store_that_checks_a_file_against_a_directory_two_parts_deep_fails_the_kit() {
    // The store is the fake in all else: it refuses a path below a file, and a file where a
    // directory is, within the first two parts of a path, and takes a clash below them.
    assert_eq!(
        run_store_kit(&ShallowRig).failed(),
        ["paths_that_escape_an_entry_are_refused"]
    );
}

#[test]
fn each_wrong_store_stub_fails_the_scenario_that_holds_it() {
    const READ_BACK: &str = "an_entry_reads_back_what_was_written";
    const ESCAPES: &str = "paths_that_escape_an_entry_are_refused";
    const LISTED: &str = "an_entry_is_listed_exactly";
    const TWO: &str = "two_entries_do_not_touch_each_other";
    const DISCARD: &str = "discard_removes_every_path_of_an_entry";
    further_stubs_fail_exactly(
        "store",
        &names(&store_scenarios::<HostileRig>()),
        &[
            (Wrong::DiscardKeepsTheFileList, &[DISCARD]),
            (Wrong::BeginRevivesADiscardedEntry, &[DISCARD]),
            (Wrong::GetIgnoresTheStage, &[TWO]),
            (Wrong::GetIgnoresThePlatform, &[TWO]),
            (Wrong::GetIgnoresTheRevision, &[TWO]),
            (Wrong::GetKeyCaseFold, &[TWO]),
            (Wrong::GetKeyUnicodeFold, &[TWO]),
            (Wrong::GetKeyTrailingSpace, &[TWO]),
            (Wrong::GetKeyTrailingDot, &[TWO]),
            (Wrong::GetSetKeyAliasesMemberSet, &[TWO]),
            (Wrong::DiscardRemovesThePathsOfEveryEntry, &[DISCARD]),
            (Wrong::DiscardRemovesSharedBytes, &[DISCARD]),
            (Wrong::GetClimbsIntoASibling, &[LISTED, ESCAPES]),
            (Wrong::OnlyFiveControlsRefused, &[ESCAPES]),
            (Wrong::DirectoriesCaseFolded, &[LISTED]),
            (Wrong::DirectoriesUnicodeFolded, &[LISTED]),
            (Wrong::DirectoriesTrailingSpaceTrimmed, &[LISTED]),
            (Wrong::TrailingDotTrimmed, &[LISTED, ESCAPES]),
            (Wrong::ThirdWriteIgnored, &[READ_BACK]),
            (Wrong::WriteOfEarlierBytesIgnored, &[READ_BACK]),
            (Wrong::IdenticalRewriteRefused, &[READ_BACK]),
            (Wrong::DeepRewriteKeepsTheOldTail, &[READ_BACK]),
            (Wrong::DeepEmptyRewriteIgnored, &[READ_BACK]),
            (Wrong::TreeIndexSurvivesDiscard, &[DISCARD]),
            (Wrong::GlobalTreeIndex, &[TWO]),
            (Wrong::PrefixNoSlashRefuses, &[ESCAPES]),
            (Wrong::TreeCheckCaseFolded, &[ESCAPES]),
            (Wrong::TreeCheckUnicodeFolded, &[ESCAPES]),
            (Wrong::TreeCheckTrimmed, &[ESCAPES]),
            (Wrong::TreeIndexByMember, &[TWO]),
        ],
        |flaw| run_store_kit(&HostileRig { flaw }),
    );
}

fn further_output_files() {
    const READ_THROUGH: &str = "an_entry_of_a_listed_directory_is_read_through_its_joined_path";
    const KINDS: &str = "a_listing_names_the_kind_of_every_entry";
    const REFUSED: &str = "listing_a_file_and_reading_a_directory_or_a_symlink_are_refused";
    const STAT: &str = "stat_tells_a_file_a_directory_a_symlink_and_nothing_apart";
    const DESCENDED: &str = "a_listed_symlink_is_never_descended";
    const SORTED: &str = "a_listing_is_sorted_relative_paths";
    const CLIMB: &str = "a_path_that_is_relative_or_climbs_is_refused";
    const CHANGED: &str = "a_read_returns_the_file_as_it_is_at_the_call";
    const UNCHANGED: &str = "no_call_changes_the_tree";
    const DOTS: &str =
        "a_dot_part_a_repeated_slash_and_a_trailing_slash_name_the_path_they_stand_in";
    further_stubs_fail_exactly(
        "output files",
        &names(&output_files_scenarios::<FakeFilesRig>()),
        &[
            (Files::LossyRead, &[READ_THROUGH]),
            (Files::EmptyFileIsMissing, &[READ_THROUGH]),
            (Files::ReadTruncatesAt4k, &[READ_THROUGH]),
            (Files::DotfilesHidden, &[KINDS, READ_THROUGH]),
            (Files::DeepEntriesOmitted, &[KINDS, READ_THROUGH]),
            (Files::LinkToDirListedAsDirectory, &[KINDS]),
            (Files::DanglingLinkOmitted, &[KINDS]),
            (Files::ListOfSymlinkToDirFollows, &[REFUSED]),
            (Files::TrailingSlashFollowsRead, &[REFUSED]),
            (Files::TrailingSlashFollowsStat, &[STAT]),
            (Files::ReadRefusesDotDotNames, &[READ_THROUGH]),
            (Files::FollowsNestedLinks, &[KINDS, DESCENDED]),
            (Files::StatBelowAFileIsAnError, &[STAT]),
            (Files::ListOfAnEmptyDirectoryIsPermanent, &[KINDS]),
            (Files::ReadTruncatesAt64k, &[READ_THROUGH]),
            (Files::ReadTruncatesAt128k, &[READ_THROUGH]),
            (Files::ReadTruncatesAt1M, &[READ_THROUGH]),
            (Files::ListCappedAt256, &[SORTED]),
            (Files::DeepEntriesOmittedBeyond5, &[READ_THROUGH]),
            (Files::ReadOfDanglingLinkIsEmpty, &[REFUSED]),
            (Files::ReadOfLinkToDirIsEmpty, &[REFUSED]),
            (Files::ListOfDanglingLinkIsEmpty, &[REFUSED]),
            (Files::ListTrailingSlashFails, &[DOTS]),
            (Files::ListTrailingSlashRelsLeadingSlash, &[DOTS]),
            (Files::RefusesDoubleSlash, &[DOTS]),
            (Files::ReadFailsOnSpaceInPath, &[READ_THROUGH]),
            (Files::ListOmitsNamesWithSpace, &[READ_THROUGH]),
            (Files::ListOmitsNonAscii, &[READ_THROUGH]),
            (Files::ListNamesPercentEncoded, &[READ_THROUGH]),
            (Files::ReadOfFileWithNewlineNameFails, &[READ_THROUGH]),
            (Files::NaturalSort, &[SORTED]),
            (Files::ReadOfEmptyFileInSubdirIsMissing, &[READ_THROUGH]),
            (Files::ReadBelowAFileAnswersTheParent, &[REFUSED]),
            (Files::ListBelowAFileIsEmpty, &[REFUSED]),
            (Files::ReadNormalisesCrlf, &[READ_THROUGH]),
            (Files::ReadStripsBom, &[READ_THROUGH]),
            (Files::ReadStripsTrailingCrlf, &[READ_THROUGH]),
            (Files::ReadTrimsTrailingNuls, &[READ_THROUGH]),
            (Files::ReadDropsTheLastByteAtA4kMultiple, &[READ_THROUGH]),
            (Files::ListNestedKindsAreFiles, &[KINDS]),
            (Files::ListDeepEntriesTwice, &[KINDS]),
            (Files::ListAddsAPhantomEntry, &[KINDS]),
            (Files::ListOmitsNestedEmptyDirectories, &[KINDS]),
            (Files::ListDeepDirectoriesHaveATrailingSlash, &[KINDS]),
            (Files::StatOfASlashedLinkIsNone, &[STAT]),
            (Files::StatOfASlashedDanglingLinkFails, &[STAT]),
            (Files::StatRefusesDotDotNames, &[STAT]),
            (Files::ListRefusesDotDotNames, &[KINDS]),
            (Files::LeadingClimbIsResolved, &[CLIMB]),
            (Files::ListFollowsASlashedLinkToADirectory, &[REFUSED]),
            (Files::ReadFollowsALinkInAnotherDirectory, &[REFUSED]),
            (Files::ReadFollowsALinkAtTheRoot, &[REFUSED]),
            (Files::ReadFollowsADeepLink, &[REFUSED]),
            (Files::ReadOfADirectoryAtTheRootIsEmpty, &[REFUSED]),
            (Files::ReadOfADeepDirectoryIsEmpty, &[REFUSED]),
            (Files::ReadDeepBelowAFileAnswersTheAncestor, &[REFUSED]),
            (Files::ListDeepBelowAFileIsEmpty, &[REFUSED]),
            (Files::StatDeepBelowAFileIsAnError, &[STAT]),
            (Files::StatBelowAMissingDirectoryIsAnError, &[STAT]),
            (Files::ListBelowAMissingDirectoryIsEmpty, &[REFUSED]),
            (Files::ReadBelowAMissingDirectoryIsEmpty, &[REFUSED]),
            (Files::ReadAcceptsADotSlashPath, &[CLIMB]),
            (Files::ReadAcceptsARootlessPath, &[CLIMB]),
            (Files::ReadTruncatesAt4M, &[READ_THROUGH]),
            (Files::ReadDropsTheLastByteAtA128kMultiple, &[READ_THROUGH]),
            (Files::ReadTrimsTrailingBlanks, &[READ_THROUGH]),
            (Files::CachesReadsByLength, &[CHANGED]),
            (Files::ReadLowercasesTheFile, &[UNCHANGED]),
            (Files::ListIncludesPrefixSiblings, &[KINDS]),
            (Files::StatFollowsALinkToClassifyIt, &[STAT]),
            (Files::SlashedFileIsNothing, &[DOTS]),
        ],
        |flaw| run_output_files_kit(&FakeFilesRig::flawed(flaw)),
    );
}

#[test]
fn each_scenario_fails_on_its_nonconforming_stub() {
    output_files();
    store();
    process_runner();
    build_system();
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
            (
                "a_dot_part_a_repeated_slash_and_a_trailing_slash_name_the_path_they_stand_in",
                Files::RefusesDotPart,
            ),
        ],
        |flaw| run_output_files_kit(&FakeFilesRig::flawed(flaw)),
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

fn store() {
    each_fails_alone(
        "store",
        &names(&store_scenarios::<FakeStoreRig>()),
        &[
            (
                "an_entry_reads_back_what_was_written",
                Store::EmptyFileIsDropped,
            ),
            ("an_entry_is_listed_exactly", Store::FilesAreNotSorted),
            (
                "a_second_begin_on_one_key_is_refused",
                Store::SecondBeginResets,
            ),
            (
                "keys_of_two_scopes_are_listed_apart",
                Store::KeysCappedAt100,
            ),
            (
                "paths_that_escape_an_entry_are_refused",
                Store::WriterAcceptsClimbingPaths,
            ),
            (
                "two_entries_do_not_touch_each_other",
                Store::EntriesShareTheBytesOfAPath,
            ),
            (
                "discard_removes_every_path_of_an_entry",
                Store::DiscardLeavesTheFirstPath,
            ),
            (
                "a_missing_entry_lists_as_empty",
                Store::KeysOfAnEmptyStoreIsAnError,
            ),
        ],
        |flaw| run_store_kit(&FakeStoreRig::flawed(flaw)),
    );
}
