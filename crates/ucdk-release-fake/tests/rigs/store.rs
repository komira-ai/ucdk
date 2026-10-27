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

impl ArtifactStore for Flawed {
    fn begin(&mut self, key: &StoreKey) -> Result<Box<dyn EntryWriter + '_>, SeamError> {
        if self.flaw == Some(Flaw::SecondBeginResets) {
            self.inner.discard(key)?;
        }
        let writer = self.inner.begin(key)?;
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
        self.inner.get(key, rel)
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
