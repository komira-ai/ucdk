// SPDX-License-Identifier: Apache-2.0
//! The self-test of the kits: for every scenario of a kit, a stub of the seam that breaks
//! that one promise and no other, and the proof that the kit then fails that scenario and only
//! that scenario. A scenario no stub can fail is a scenario that checks nothing.

mod rigs;

use rigs::hostile::{HostileRig, Wrong};
use rigs::loose::LooseRig;
use rigs::output_files::{FakeFilesRig, Flaw as Files};
use rigs::store::{FakeStoreRig, Flaw as Store};
use rigs::{each_fails_alone, further_stubs_fail_exactly, names};
use ucdk_release_seams::kit::{
    output_files_scenarios, run_output_files_kit, run_store_kit, store_scenarios,
};

#[test]
fn each_further_output_files_stub_fails_the_scenarios_that_hold_it() {
    further_output_files();
}

#[test]
fn each_further_store_stub_fails_the_scenarios_that_hold_it() {
    further_store();
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
            (Store::PathsAreCaseFolded, &[LISTED]),
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
            (Store::TrailingSpaceTrimmed, &[LISTED]),
            (Store::UnicodeFolded, &[LISTED]),
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
            (Wrong::TrailingDotTrimmed, &[LISTED]),
            (Wrong::ThirdWriteIgnored, &[READ_BACK]),
            (Wrong::WriteOfEarlierBytesIgnored, &[READ_BACK]),
            (Wrong::IdenticalRewriteRefused, &[READ_BACK]),
            (Wrong::DeepRewriteKeepsTheOldTail, &[READ_BACK]),
            (Wrong::DeepEmptyRewriteIgnored, &[READ_BACK]),
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
    const DOTS: &str =
        "a_dot_part_a_repeated_slash_and_a_trailing_slash_name_the_path_they_stand_in";
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
        ],
        |flaw| run_output_files_kit(&FakeFilesRig::flawed(flaw)),
    );
}

#[test]
fn each_scenario_fails_on_its_nonconforming_stub() {
    output_files();
    store();
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
