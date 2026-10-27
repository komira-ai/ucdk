// SPDX-License-Identifier: Apache-2.0
//! A store that is the fake in everything but one promise: it lets a file and a directory of one
//! name share an entry, which the paths of an entry, a tree, cannot.

use std::collections::BTreeMap;

use ucdk_release::store::StoreKey;
use ucdk_release_seams::kit::StoreRig;
use ucdk_release_seams::{ArtifactStore, EntryWriter, SeamError, valid_rel};

pub struct LooseRig;

impl StoreRig for LooseRig {
    type Store = Loose;

    fn store(&self) -> Loose {
        Loose {
            entries: BTreeMap::new(),
        }
    }
}

pub struct Loose {
    entries: BTreeMap<StoreKey, BTreeMap<String, Vec<u8>>>,
}

struct Writer<'a>(&'a mut BTreeMap<String, Vec<u8>>);

impl EntryWriter for Writer<'_> {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        if !valid_rel(rel) || rel.contains('\\') || rel.chars().any(char::is_control) {
            return Err(SeamError::Refused(
                ucdk_release_proto::v1::Reason::SetMember,
            ));
        }
        self.0.insert(rel.to_owned(), bytes.to_vec());
        Ok(())
    }
}

impl ArtifactStore for Loose {
    fn begin(&mut self, key: &StoreKey) -> Result<Box<dyn EntryWriter + '_>, SeamError> {
        if self.entries.contains_key(key) {
            return Err(SeamError::Refused(
                ucdk_release_proto::v1::Reason::SetMember,
            ));
        }
        Ok(Box::new(Writer(
            self.entries.entry(key.clone()).or_default(),
        )))
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
