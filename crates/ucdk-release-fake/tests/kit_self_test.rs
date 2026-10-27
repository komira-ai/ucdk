// SPDX-License-Identifier: Apache-2.0
//! The self-test of the kits: for every scenario of a kit, a stub of the seam that breaks
//! that one promise and no other, and the proof that the kit then fails that scenario and only
//! that scenario. A scenario no stub can fail is a scenario that checks nothing.

mod rigs;

use rigs::hostile::{HostileRig, Wrong};
use rigs::loose::LooseRig;
use rigs::store::{FakeStoreRig, Flaw as Store};
use rigs::{each_fails_alone, further_stubs_fail_exactly, names};
use ucdk_release_seams::kit::{run_store_kit, store_scenarios};

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

#[test]
fn each_scenario_fails_on_its_nonconforming_stub() {
    store();
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
