// SPDX-License-Identifier: Apache-2.0
//! The store rig: the fake store, and the stubs that break one promise of the store.

use ucdk_release::store::StoreKey;
use ucdk_release_fake::FakeStore;
use ucdk_release_seams::kit::StoreRig;
use ucdk_release_seams::{ArtifactStore, EntryWriter, SeamError};

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
        let writer = self.inner.begin(key)?;
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
        let mut files = self.inner.files(key)?;
        if self.flaw == Some(Flaw::FilesAreNotSorted) {
            files.reverse();
        }
        Ok(files)
    }

    fn get(&mut self, key: &StoreKey, rel: &str) -> Result<Option<Vec<u8>>, SeamError> {
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
                _ => {}
            }
            bytes
        }))
    }

    fn discard(&mut self, key: &StoreKey) -> Result<(), SeamError> {
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
