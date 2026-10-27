// SPDX-License-Identifier: Apache-2.0
//! The artifact store in memory: a tree of files under each key.

use std::collections::BTreeMap;

use ucdk_release::store::StoreKey;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{ArtifactStore, EntryWriter, SeamError, valid_rel};

type Entry = BTreeMap<String, Vec<u8>>;

#[derive(Debug, Default)]
pub struct FakeStore {
    entries: BTreeMap<StoreKey, Entry>,
}

impl FakeStore {
    pub fn new() -> Self {
        Self::default()
    }
}

struct Writer<'a> {
    entry: &'a mut Entry,
}

impl EntryWriter for Writer<'_> {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        if !valid_rel(rel) || rel.contains('\\') || rel.chars().any(char::is_control) {
            return Err(SeamError::Refused(Reason::SetMember));
        }
        // The paths form a tree: no directory of `rel` is a file, and `rel` is no directory.
        let below = format!("{rel}/");
        let is_directory = self
            .entry
            .range(below.clone()..)
            .next()
            .is_some_and(|(path, _)| path.starts_with(&below));
        let under_a_file = rel
            .match_indices('/')
            .any(|(at, _)| self.entry.contains_key(&rel[..at]));
        if is_directory || under_a_file {
            return Err(SeamError::Refused(Reason::SetMember));
        }
        self.entry.insert(rel.to_owned(), bytes.to_vec());
        Ok(())
    }
}

impl ArtifactStore for FakeStore {
    fn begin(&mut self, key: &StoreKey) -> Result<Box<dyn EntryWriter + '_>, SeamError> {
        if self.entries.contains_key(key) {
            return Err(SeamError::Refused(Reason::SetMember));
        }
        let entry = self.entries.entry(key.clone()).or_default();
        Ok(Box::new(Writer { entry }))
    }

    fn files(&mut self, key: &StoreKey) -> Result<Vec<String>, SeamError> {
        Ok(self
            .entries
            .get(key)
            .map(|entry| entry.keys().cloned().collect())
            .unwrap_or_default())
    }

    fn get(&mut self, key: &StoreKey, rel: &str) -> Result<Option<Vec<u8>>, SeamError> {
        Ok(self
            .entries
            .get(key)
            .and_then(|entry| entry.get(rel))
            .cloned())
    }

    fn discard(&mut self, key: &StoreKey) -> Result<(), SeamError> {
        self.entries.remove(key);
        Ok(())
    }

    fn keys(&mut self) -> Result<Vec<StoreKey>, SeamError> {
        Ok(self.entries.keys().cloned().collect())
    }
}
