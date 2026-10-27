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
    /// `keys` lists one key for each member name.
    KeysCollapseByMemberName,
    /// A writer that accepts a path that climbs, and writes it somewhere else.
    WriterAcceptsClimbingPaths,
    /// `get` looks in every entry for the path and answers the first it finds.
    GetIgnoresTheKey,
    /// `discard` leaves the first path of the entry in place.
    DiscardLeavesTheFirstPath,
    /// `files` of an entry that does not exist is an error.
    MissingEntryIsAnError,
    /// `get` of a nested path answers nothing, though it was written.
    GetLosesNestedPaths,
    /// A writer that takes a path with a trailing slash and writes nothing.
    WriterAcceptsATrailingSlash,
    /// A writer that takes a path with a `.` part and writes nothing.
    WriterAcceptsADotPart,
    /// A writer that takes a path with a NUL or a tab in it and writes nothing.
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
        } else {
            self.inner.put_file(rel, kept)
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
}

impl StoreRig for FakeStoreRig {
    type Store = Flawed;

    fn store(&self) -> Flawed {
        Flawed {
            inner: FakeStore::new(),
            flaw: self.flaw,
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
        let writer = self.inner.begin(key)?;
        if matches!(
            self.flaw,
            Some(
                Flaw::DeepPathsDropped
                    | Flaw::PutTruncatesAt64k
                    | Flaw::AppendOnRewrite
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
                Some(|rel, _| rel.contains('\0') || rel.contains('\t'))
            }
            Some(Flaw::EmptyFileIsDropped) => Some(|_, bytes| bytes.is_empty()),
            _ => None,
        };
        if let Some(drops) = drops {
            return Ok(Box::new(Dropping {
                inner: writer,
                drops,
            }));
        }
        if self.flaw == Some(Flaw::WriterAcceptsClimbingPaths) {
            return Ok(Box::new(Climbing(writer)));
        }
        Ok(writer)
    }

    fn files(&mut self, key: &StoreKey) -> Result<Vec<String>, SeamError> {
        if self.flaw == Some(Flaw::MissingEntryIsAnError) && !self.inner.keys()?.contains(key) {
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
        Ok(files)
    }

    fn get(&mut self, key: &StoreKey, rel: &str) -> Result<Option<Vec<u8>>, SeamError> {
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
        let found = self.inner.get(key, rel)?;
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
        if self.flaw == Some(Flaw::KeysCollapseByMemberName) {
            let mut seen: Vec<String> = Vec::new();
            keys.retain(|key| {
                let member = key
                    .as_str()
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let fresh = !seen.contains(&member);
                seen.push(member);
                fresh
            });
        }
        Ok(keys)
    }
}
