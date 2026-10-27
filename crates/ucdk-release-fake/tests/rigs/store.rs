// SPDX-License-Identifier: Apache-2.0
//! The store rig: the fake store, and the stubs that break one promise of the store.

use std::collections::BTreeMap;

use ucdk_release::store::StoreKey;
use ucdk_release_fake::FakeStore;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::kit::StoreRig;
use ucdk_release_seams::{ArtifactStore, EntryWriter, SeamError, valid_rel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// Lists the paths of an entry in the reverse of sorted order.
    FilesAreNotSorted,
    /// A second `begin` on a key empties the entry and succeeds.
    SecondBeginResets,
    /// `keys` lists one key for two keys that differ only in the generation.
    KeysCollapseAcrossGenerations,
    /// A writer that accepts a path that climbs, and writes it somewhere else.
    WriterAcceptsClimbingPaths,
    /// `get` looks in every entry for the path and answers the first it finds.
    GetIgnoresTheKey,
    /// `get` of a path the entry holds answers the bytes of the first entry that holds the path.
    EntriesShareTheBytesOfAPath,
    /// `discard` leaves the first path of the entry in place.
    DiscardLeavesTheFirstPath,
    /// `files` of an entry that was never begun is an error.
    MissingEntryIsAnError,
    /// `get` of a nested path answers nothing, though it was written.
    GetLosesNestedPaths,
    /// A writer that takes a path with a trailing slash and writes nothing.
    WriterAcceptsATrailingSlash,
    /// A writer that takes a path with a `.` part and writes nothing.
    WriterAcceptsADotPart,
    /// A writer that takes a path with a control character in it (a NUL, a tab, a carriage
    /// return, a DEL) and writes nothing.
    WriterAcceptsControlCharacters,
    /// A writer that takes an empty file and writes nothing.
    EmptyFileIsDropped,
    /// `get` turns bytes that are not text into replacement characters.
    GetIsLossy,
    /// `get` answers no more than the first 4096 bytes.
    GetTruncatesAt4k,
    /// `get` of a directory of the entry answers an empty file.
    GetOfADirectoryIsEmpty,
    /// `get` resolves a path with `..` parts in it and answers the file it comes to.
    GetResolvesClimbs,
    /// `discard` of one entry discards every entry.
    DiscardWipesEveryEntry,
    /// `discard` of an entry that is not there is an error.
    DiscardOfAMissingEntryIsAnError,
    /// `files` of a key lists the files of every key that starts with it.
    FilesMatchedByKeyPrefix,
    /// Paths are folded to lower case, so `X` and `x` are one file.
    PathsAreCaseFolded,
    /// `get` of a path with a trailing slash reads the file the path names without it.
    GetTakesATrailingSlash,
    /// An entry that was begun and holds no file is no entry: a second `begin` succeeds.
    EmptyEntryIsNoEntry,
    /// A writer takes a path three or more directories deep, checks it, and writes nothing.
    DeepPathsDropped,
    /// `files` leaves out every path three or more parts deep.
    DeepPathsNotListed,
    /// `get` of a path three or more parts deep answers nothing.
    DeepPathsGetNone,
    /// `discard` leaves the paths three or more parts deep in place.
    DiscardDeepSurvives,
    /// A writer keeps no more than the first 65,536 bytes of a file.
    PutTruncatesAt64k,
    /// `get` answers no more than the first 65,536 bytes.
    GetTruncatesAt64k,
    /// A second write of one path appends to the first.
    AppendOnRewrite,
    /// `discard` of a key discards every key that it is a prefix of.
    DiscardPrefixSiblings,
    /// `begin` is refused when a key that starts with the new key exists.
    BeginRefusedOnExistingLongerKey,
    /// `keys` leaves out an entry that holds no file.
    KeysOmitsEmptyEntries,
    /// After a path was refused, the writer refuses every later path.
    WriterPoisonedAfterRefusal,
    /// `files` leaves out every path that has a part starting with a dot.
    DotfilesHidden,
    /// Trailing spaces of a path are trimmed, so `a ` and `a` are one file.
    TrailingSpaceTrimmed,
    /// A path is put in one Unicode composition, so `e` with an accent and the single character
    /// are one file.
    UnicodeFolded,
    /// `files` lists no more than 100 paths.
    FilesCappedAt100,
    /// `keys` lists no more than 100 keys.
    KeysCappedAt100,
    /// A second write of one path that is shorter than the first keeps the tail of the first.
    RewriteKeepsTheOldTail,
    /// A writer that takes a path that ends in a `.` or a `..` part and writes nothing.
    WriterAcceptsAPathEndingInADotPart,
    /// A key of the set document is the key of the member called `set`.
    SetKeyAliasesMemberSet,
    /// `begin` of the key of a set document is refused.
    SetKeyRefused,
    /// Two keys that differ only in the platform are one entry.
    PlatformIgnored,
    /// Two keys that differ only in the revision are one entry.
    RevisionIgnored,
    /// Keys are folded to lower case, so two keys that differ only in case are one entry.
    KeysAreCaseFolded,
    /// Keys are put in one Unicode composition, so two that differ only in it are one entry.
    KeysAreUnicodeFolded,
    /// Trailing spaces of a key are trimmed, so two keys that differ in them are one entry.
    KeysTrailingSpaceTrimmed,
    /// Characters of a key that are not ASCII are all one character, so two keys that differ
    /// only in them are one entry.
    KeysNonAsciiAlike,
    /// `discard` of a key discards every entry with the same member name.
    DiscardSameMember,
    /// `discard` of an entry that is not there discards every entry.
    DiscardMissingWipes,
    /// `discard` of an entry that is not there discards the entry begun last.
    DiscardMissingRemovesLast,
    /// `discard` of an entry that is not there begins it, empty.
    DiscardMissingCreatesEmpty,
    /// A writer that refuses a path that climbs, after it wrote the bytes over the file that the
    /// path names without its climb.
    RefusedPutOverwrites,
    /// `get` of a path the entry does not hold answers the same path of a longer key.
    GetPrefixFallback,
    /// `get` of a path the entry does not hold answers the path in lower case.
    GetCaseFallback,
    /// `get` of a path the entry does not hold answers the path in the other Unicode composition.
    GetUnicodeFallback,
    /// A second write of a nested path that is shorter than the first keeps the old tail.
    NestedRewriteKeepsTheOldTail,
    /// A writer that takes an empty file at a nested path and writes nothing.
    NestedEmptyIsDropped,
    /// A path that was written empty keeps no later bytes.
    EmptyThenNonEmptyIgnored,
    /// A second write of a nested path that is empty is ignored.
    NestedEmptyRewriteIgnored,
    /// `files` sorts runs of digits by their value, so `f2` comes before `f10`.
    FilesNaturalSort,
    /// `files` sorts by UTF-16 code unit, so a character above U+FFFF comes before U+FF5E.
    FilesUtf16Sort,
}

fn deep(rel: &str) -> bool {
    rel.split('/').count() >= 3
}

/// The path as the stub that folds paths keeps it.
fn folded(flaw: Option<Flaw>, rel: &str) -> String {
    match flaw {
        Some(Flaw::TrailingSpaceTrimmed) => rel.trim_end_matches(' ').to_owned(),
        Some(Flaw::UnicodeFolded) => rel.replace("e\u{301}", "\u{e9}"),
        _ => rel.to_owned(),
    }
}

/// The key as a store that confuses keys of one kind compares it, or `None` for a store that
/// confuses none.
fn alias(flaw: Option<Flaw>, key: &StoreKey) -> Option<String> {
    let text = key.as_str();
    let without = |index: usize| {
        text.split('/')
            .enumerate()
            .filter(|(at, _)| *at != index)
            .map(|(_, part)| part)
            .collect::<Vec<_>>()
            .join("/")
    };
    match flaw? {
        Flaw::SetKeyAliasesMemberSet => Some(text.replace("/m/", "/")),
        Flaw::PlatformIgnored => Some(without(1)),
        Flaw::RevisionIgnored => Some(without(2)),
        Flaw::KeysAreCaseFolded => Some(text.to_lowercase()),
        Flaw::KeysAreUnicodeFolded => Some(text.replace("e\u{301}", "\u{e9}")),
        Flaw::KeysTrailingSpaceTrimmed => Some(text.trim_end_matches(' ').to_owned()),
        Flaw::KeysNonAsciiAlike => Some(
            text.chars()
                .map(|c| if c.is_ascii() { c } else { '?' })
                .collect(),
        ),
        _ => None,
    }
}

/// A path as a natural sort reads it: a run of digits stands for its value.
fn natural(rel: &str) -> Vec<(u8, u128, String)> {
    let mut tokens = Vec::new();
    let mut rest = rel;
    while !rest.is_empty() {
        let digits = rest.starts_with(|c: char| c.is_ascii_digit());
        let end = rest
            .find(|c: char| c.is_ascii_digit() != digits)
            .unwrap_or(rest.len());
        let (run, tail) = rest.split_at(end);
        tokens.push(if digits {
            (0, run.parse().unwrap_or(0), String::new())
        } else {
            (1, 0, run.to_owned())
        });
        rest = tail;
    }
    tokens
}

/// A writer for the stubs that change what a write does.
struct Shaping<'a> {
    inner: Box<dyn EntryWriter + 'a>,
    flaw: Option<Flaw>,
    refused: bool,
    written: BTreeMap<String, Vec<u8>>,
}

impl EntryWriter for Shaping<'_> {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        if self.flaw == Some(Flaw::WriterPoisonedAfterRefusal) && self.refused {
            return Err(SeamError::Refused(Reason::SetMember));
        }
        let rel = folded(self.flaw, rel);
        let rel = rel.as_str();
        if self.flaw == Some(Flaw::DeepPathsDropped) && deep(rel) {
            if !valid_rel(rel) || rel.contains('\\') || rel.chars().any(char::is_control) {
                return Err(SeamError::Refused(Reason::SetMember));
            }
            return Ok(());
        }
        let held = self.written.get(rel);
        let ignored = match self.flaw {
            Some(Flaw::EmptyThenNonEmptyIgnored) => {
                !bytes.is_empty() && held.is_some_and(Vec::is_empty)
            }
            Some(Flaw::NestedEmptyRewriteIgnored) => {
                bytes.is_empty() && rel.contains('/') && held.is_some()
            }
            _ => false,
        };
        if ignored {
            return Ok(());
        }
        let kept = match self.flaw {
            Some(Flaw::PutTruncatesAt64k) => &bytes[..bytes.len().min(65_536)],
            _ => bytes,
        };
        let result = if self.flaw == Some(Flaw::AppendOnRewrite) {
            let mut all = self.written.get(rel).cloned().unwrap_or_default();
            all.extend_from_slice(kept);
            let put = self.inner.put_file(rel, &all);
            if put.is_ok() {
                self.written.insert(rel.to_owned(), all);
            }
            put
        } else if self.flaw == Some(Flaw::RewriteKeepsTheOldTail)
            || (self.flaw == Some(Flaw::NestedRewriteKeepsTheOldTail) && rel.contains('/'))
        {
            let mut all = kept.to_vec();
            if let Some(old) = self.written.get(rel) {
                all.extend_from_slice(old.get(kept.len()..).unwrap_or_default());
            }
            let put = self.inner.put_file(rel, &all);
            if put.is_ok() {
                self.written.insert(rel.to_owned(), all);
            }
            put
        } else {
            let put = self.inner.put_file(rel, kept);
            if put.is_ok() {
                self.written.insert(rel.to_owned(), kept.to_vec());
            }
            put
        };
        self.refused |= result.is_err();
        result
    }
}

pub struct FakeStoreRig {
    pub flaw: Option<Flaw>,
}

impl FakeStoreRig {
    pub fn conforming() -> Self {
        Self { flaw: None }
    }

    pub fn flawed(flaw: Flaw) -> Self {
        Self { flaw: Some(flaw) }
    }
}

pub struct Flawed {
    inner: FakeStore,
    flaw: Option<Flaw>,
    /// The key last begun.
    last: Option<StoreKey>,
    /// Every key ever begun, discarded or not.
    begun: Vec<StoreKey>,
}

impl Flawed {
    /// The entry a key names in a store that confuses keys of one kind: the one that already
    /// holds the place, if there is one.
    fn resolve(&mut self, key: &StoreKey) -> Result<StoreKey, SeamError> {
        if let Some(wanted) = alias(self.flaw, key) {
            for other in self.inner.keys()? {
                if alias(self.flaw, &other).as_deref() == Some(wanted.as_str()) {
                    return Ok(other);
                }
            }
        }
        Ok(key.clone())
    }

    /// What `get` answers, in the stubs that answer for a path the entry does not hold.
    fn fallback(&mut self, key: &StoreKey, rel: &str) -> Result<Option<Vec<u8>>, SeamError> {
        match self.flaw {
            Some(Flaw::GetPrefixFallback) => {
                for other in self.inner.keys()? {
                    if other != *key
                        && other.as_str().starts_with(key.as_str())
                        && let Some(bytes) = self.inner.get(&other, rel)?
                    {
                        return Ok(Some(bytes));
                    }
                }
                Ok(None)
            }
            Some(Flaw::GetCaseFallback) => self.inner.get(key, &rel.to_lowercase()),
            Some(Flaw::GetUnicodeFallback) => {
                if let Some(bytes) = self.inner.get(key, &rel.replace("e\u{301}", "\u{e9}"))? {
                    return Ok(Some(bytes));
                }
                self.inner.get(key, &rel.replace('\u{e9}', "e\u{301}"))
            }
            _ => Ok(None),
        }
    }
}

impl StoreRig for FakeStoreRig {
    type Store = Flawed;

    fn store(&self) -> Flawed {
        Flawed {
            inner: FakeStore::new(),
            flaw: self.flaw,
            last: None,
            begun: Vec::new(),
        }
    }
}

struct Climbing<'a>(Box<dyn EntryWriter + 'a>);

impl EntryWriter for Climbing<'_> {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        if rel.starts_with("../") {
            self.0.put_file("escaped", bytes)
        } else {
            self.0.put_file(rel, bytes)
        }
    }
}

/// A writer that writes every path in lower case.
struct Folding<'a>(Box<dyn EntryWriter + 'a>);

impl EntryWriter for Folding<'_> {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        self.0.put_file(&rel.to_lowercase(), bytes)
    }
}

/// A writer that refuses a path that climbs, after it wrote the bytes over the file that the
/// path names without its climb.
struct WritesBeforeRefusing<'a>(Box<dyn EntryWriter + 'a>);

impl EntryWriter for WritesBeforeRefusing<'_> {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        let refused = self.0.put_file(rel, bytes);
        if let Some(name) = rel.strip_prefix("../")
            && refused.is_err()
        {
            self.0.put_file(name, bytes).ok();
        }
        refused
    }
}

/// A writer that takes, and drops, what `drops` says it takes, and passes on the rest.
struct Dropping<'a> {
    inner: Box<dyn EntryWriter + 'a>,
    drops: fn(&str, &[u8]) -> bool,
}

impl EntryWriter for Dropping<'_> {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        if (self.drops)(rel, bytes) {
            Ok(())
        } else {
            self.inner.put_file(rel, bytes)
        }
    }
}

impl ArtifactStore for Flawed {
    fn begin(&mut self, key: &StoreKey) -> Result<Box<dyn EntryWriter + '_>, SeamError> {
        let key = &self.resolve(key)?;
        if self.flaw == Some(Flaw::SetKeyRefused)
            && key.as_str().ends_with("/set")
            && !key.as_str().contains("/m/")
        {
            return Err(SeamError::Refused(Reason::SetMember));
        }
        if self.flaw == Some(Flaw::SecondBeginResets) {
            self.inner.discard(key)?;
        }
        if self.flaw == Some(Flaw::EmptyEntryIsNoEntry) && self.inner.files(key)?.is_empty() {
            self.inner.discard(key)?;
        }
        if self.flaw == Some(Flaw::BeginRefusedOnExistingLongerKey)
            && self
                .inner
                .keys()?
                .iter()
                .any(|other| other != key && other.as_str().starts_with(key.as_str()))
        {
            return Err(SeamError::Refused(Reason::SetMember));
        }
        self.last = Some(key.clone());
        self.begun.push(key.clone());
        let writer = self.inner.begin(key)?;
        if matches!(
            self.flaw,
            Some(
                Flaw::DeepPathsDropped
                    | Flaw::PutTruncatesAt64k
                    | Flaw::AppendOnRewrite
                    | Flaw::RewriteKeepsTheOldTail
                    | Flaw::NestedRewriteKeepsTheOldTail
                    | Flaw::EmptyThenNonEmptyIgnored
                    | Flaw::NestedEmptyRewriteIgnored
                    | Flaw::WriterPoisonedAfterRefusal
                    | Flaw::TrailingSpaceTrimmed
                    | Flaw::UnicodeFolded
            )
        ) {
            return Ok(Box::new(Shaping {
                inner: writer,
                flaw: self.flaw,
                refused: false,
                written: BTreeMap::new(),
            }));
        }
        if self.flaw == Some(Flaw::PathsAreCaseFolded) {
            return Ok(Box::new(Folding(writer)));
        }
        let drops: Option<fn(&str, &[u8]) -> bool> = match self.flaw {
            Some(Flaw::WriterAcceptsATrailingSlash) => Some(|rel, _| rel.ends_with('/')),
            Some(Flaw::WriterAcceptsADotPart) => {
                Some(|rel, _| rel != "." && rel.split('/').any(|part| part == "."))
            }
            Some(Flaw::WriterAcceptsControlCharacters) => {
                Some(|rel, _| rel.chars().any(char::is_control))
            }
            Some(Flaw::WriterAcceptsAPathEndingInADotPart) => {
                Some(|rel, _| rel.ends_with("/.") || rel.ends_with("/.."))
            }
            Some(Flaw::EmptyFileIsDropped) => Some(|_, bytes| bytes.is_empty()),
            Some(Flaw::NestedEmptyIsDropped) => {
                Some(|rel, bytes| bytes.is_empty() && rel.contains('/'))
            }
            _ => None,
        };
        if let Some(drops) = drops {
            return Ok(Box::new(Dropping {
                inner: writer,
                drops,
            }));
        }
        if self.flaw == Some(Flaw::RefusedPutOverwrites) {
            return Ok(Box::new(WritesBeforeRefusing(writer)));
        }
        if self.flaw == Some(Flaw::WriterAcceptsClimbingPaths) {
            return Ok(Box::new(Climbing(writer)));
        }
        Ok(writer)
    }

    fn files(&mut self, key: &StoreKey) -> Result<Vec<String>, SeamError> {
        let key = &self.resolve(key)?;
        if self.flaw == Some(Flaw::MissingEntryIsAnError) && !self.begun.contains(key) {
            return Err(SeamError::Permanent(format!("{key} does not exist")));
        }
        if self.flaw == Some(Flaw::FilesMatchedByKeyPrefix) {
            let mut all = Vec::new();
            for other in self.inner.keys()? {
                if other.as_str().starts_with(key.as_str()) {
                    all.extend(self.inner.files(&other)?);
                }
            }
            all.sort();
            all.dedup();
            return Ok(all);
        }
        let mut files = self.inner.files(key)?;
        if self.flaw == Some(Flaw::FilesAreNotSorted) {
            files.reverse();
        }
        if self.flaw == Some(Flaw::DeepPathsNotListed) {
            files.retain(|rel| !deep(rel));
        }
        if self.flaw == Some(Flaw::DotfilesHidden) {
            files.retain(|rel| !rel.split('/').any(|part| part.starts_with('.')));
        }
        if self.flaw == Some(Flaw::FilesCappedAt100) {
            files.truncate(100);
        }
        if self.flaw == Some(Flaw::FilesNaturalSort) {
            files.sort_by_key(|rel| (natural(rel), rel.clone()));
        }
        if self.flaw == Some(Flaw::FilesUtf16Sort) {
            files.sort_by_key(|rel| rel.encode_utf16().collect::<Vec<u16>>());
        }
        Ok(files)
    }

    fn get(&mut self, key: &StoreKey, rel: &str) -> Result<Option<Vec<u8>>, SeamError> {
        let key = &self.resolve(key)?;
        let kept;
        let rel = match self.flaw {
            Some(Flaw::PathsAreCaseFolded) => {
                kept = rel.to_lowercase();
                kept.as_str()
            }
            Some(Flaw::GetTakesATrailingSlash) => rel.trim_end_matches('/'),
            Some(Flaw::TrailingSpaceTrimmed | Flaw::UnicodeFolded) => {
                kept = folded(self.flaw, rel);
                kept.as_str()
            }
            _ => rel,
        };
        if self.flaw == Some(Flaw::DeepPathsGetNone) && deep(rel) {
            return Ok(None);
        }
        if self.flaw == Some(Flaw::GetIgnoresTheKey) {
            for other in self.inner.keys()? {
                if let Some(bytes) = self.inner.get(&other, rel)? {
                    return Ok(Some(bytes));
                }
            }
            return Ok(None);
        }
        if self.flaw == Some(Flaw::EntriesShareTheBytesOfAPath)
            && self.inner.get(key, rel)?.is_some()
        {
            for other in self.inner.keys()? {
                if let Some(bytes) = self.inner.get(&other, rel)? {
                    return Ok(Some(bytes));
                }
            }
        }
        if self.flaw == Some(Flaw::GetLosesNestedPaths) && rel.contains('/') {
            return Ok(None);
        }
        if self.flaw == Some(Flaw::GetResolvesClimbs) && rel.contains("..") {
            let mut parts: Vec<&str> = Vec::new();
            for part in rel.split('/') {
                if part == ".." {
                    parts.pop();
                } else {
                    parts.push(part);
                }
            }
            return self.inner.get(key, &parts.join("/"));
        }
        let mut found = self.inner.get(key, rel)?;
        if found.is_none() {
            found = self.fallback(key, rel)?;
        }
        if found.is_none()
            && self.flaw == Some(Flaw::GetOfADirectoryIsEmpty)
            && self
                .inner
                .files(key)?
                .iter()
                .any(|file| file.starts_with(&format!("{rel}/")))
        {
            return Ok(Some(Vec::new()));
        }
        Ok(found.map(|mut bytes| {
            match self.flaw {
                Some(Flaw::GetIsLossy) => {
                    bytes = String::from_utf8_lossy(&bytes).into_owned().into_bytes();
                }
                Some(Flaw::GetTruncatesAt4k) => bytes.truncate(4096),
                Some(Flaw::GetTruncatesAt64k) => bytes.truncate(65_536),
                _ => {}
            }
            bytes
        }))
    }

    fn discard(&mut self, key: &StoreKey) -> Result<(), SeamError> {
        let key = &self.resolve(key)?;
        if !self.inner.keys()?.contains(key) {
            match self.flaw {
                Some(Flaw::DiscardMissingWipes) => {
                    for each in self.inner.keys()? {
                        self.inner.discard(&each)?;
                    }
                    return Ok(());
                }
                Some(Flaw::DiscardMissingRemovesLast) => {
                    if let Some(last) = self.last.clone() {
                        self.inner.discard(&last)?;
                    }
                    return Ok(());
                }
                Some(Flaw::DiscardMissingCreatesEmpty) => {
                    drop(self.inner.begin(key)?);
                    return Ok(());
                }
                _ => {}
            }
        }
        if self.flaw == Some(Flaw::DiscardSameMember) {
            let member = |key: &StoreKey| {
                key.as_str()
                    .rsplit("/m/")
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            };
            let wanted = member(key);
            for other in self.inner.keys()? {
                if member(&other) == wanted {
                    self.inner.discard(&other)?;
                }
            }
            return Ok(());
        }
        if self.flaw == Some(Flaw::DiscardWipesEveryEntry) {
            for each in self.inner.keys()? {
                self.inner.discard(&each)?;
            }
            return Ok(());
        }
        if self.flaw == Some(Flaw::DiscardOfAMissingEntryIsAnError)
            && !self.inner.keys()?.contains(key)
        {
            return Err(SeamError::Permanent(format!("{key} does not exist")));
        }
        if self.flaw == Some(Flaw::DiscardPrefixSiblings) {
            for other in self.inner.keys()? {
                if other.as_str().starts_with(key.as_str()) {
                    self.inner.discard(&other)?;
                }
            }
            return Ok(());
        }
        if self.flaw == Some(Flaw::DiscardDeepSurvives) {
            let mut left = Vec::new();
            for rel in self.inner.files(key)?.into_iter().filter(|rel| deep(rel)) {
                if let Some(bytes) = self.inner.get(key, &rel)? {
                    left.push((rel, bytes));
                }
            }
            self.inner.discard(key)?;
            if !left.is_empty() {
                let mut writer = self.inner.begin(key)?;
                for (rel, bytes) in left {
                    writer.put_file(&rel, &bytes)?;
                }
            }
            return Ok(());
        }
        if self.flaw == Some(Flaw::DiscardLeavesTheFirstPath) {
            let first = self.inner.files(key)?.into_iter().next();
            let bytes = match &first {
                Some(rel) => self.inner.get(key, rel)?,
                None => None,
            };
            self.inner.discard(key)?;
            if let (Some(rel), Some(bytes)) = (first, bytes) {
                self.inner.begin(key)?.put_file(&rel, &bytes)?;
            }
            return Ok(());
        }
        self.inner.discard(key)
    }

    fn keys(&mut self) -> Result<Vec<StoreKey>, SeamError> {
        let mut keys = self.inner.keys()?;
        if self.flaw == Some(Flaw::KeysOmitsEmptyEntries) {
            let mut kept = Vec::new();
            for key in keys {
                if !self.inner.files(&key)?.is_empty() {
                    kept.push(key);
                }
            }
            keys = kept;
        }
        if self.flaw == Some(Flaw::KeysCappedAt100) {
            keys.truncate(100);
        }
        if self.flaw == Some(Flaw::KeysCollapseAcrossGenerations) {
            let mut seen: Vec<String> = Vec::new();
            keys.retain(|key| {
                let without_generation = key
                    .as_str()
                    .split('/')
                    .enumerate()
                    .filter(|(at, _)| *at != 3)
                    .map(|(_, part)| part)
                    .collect::<Vec<_>>()
                    .join("/");
                let fresh = !seen.contains(&without_generation);
                seen.push(without_generation);
                fresh
            });
        }
        Ok(keys)
    }
}
