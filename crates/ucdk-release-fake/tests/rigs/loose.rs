// SPDX-License-Identifier: Apache-2.0
//! Stores that are the fake in everything but one promise about the tree of the paths of an
//! entry: one lets a file and a directory of one name share an entry, which the paths of an
//! entry, a tree, cannot, and one keeps the promise for the first two parts of a path and no
//! further.

use std::collections::BTreeMap;

use ucdk_release::store::StoreKey;
use ucdk_release_seams::kit::StoreRig;
use ucdk_release_seams::{ArtifactStore, EntryWriter, SeamError, valid_rel};

/// A store with no check of a file against a directory.
pub struct LooseRig;

/// A store that checks a file against a directory in the first two parts of a path only.
pub struct ShallowRig;

impl StoreRig for LooseRig {
    type Store = Loose;

    fn store(&self) -> Loose {
        Loose {
            entries: BTreeMap::new(),
            depth: 0,
        }
    }
}

impl StoreRig for ShallowRig {
    type Store = Loose;

    fn store(&self) -> Loose {
        Loose {
            entries: BTreeMap::new(),
            depth: 2,
        }
    }
}

pub struct Loose {
    entries: BTreeMap<StoreKey, BTreeMap<String, Vec<u8>>>,
    /// How many parts of a path the check of a file against a directory looks at; none, no check.
    depth: usize,
}

struct Writer<'a> {
    files: &'a mut BTreeMap<String, Vec<u8>>,
    depth: usize,
}

/// Whether `new` is below a file that `old` names, or is a directory of `old`'s path taken as a
/// file, as far as `depth` parts of each go.
fn clashes_within(depth: usize, new: &str, old: &str) -> bool {
    let (new, old): (Vec<&str>, Vec<&str>) = (new.split('/').collect(), old.split('/').collect());
    let below_a_file = old.len() <= depth && new.len() > old.len() && new[..old.len()] == old[..];
    let a_directory = new.len() <= depth && new.len() < old.len() && old[..new.len()] == new[..];
    below_a_file || a_directory
}

impl EntryWriter for Writer<'_> {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        if !valid_rel(rel) || rel.contains('\\') || rel.chars().any(char::is_control) {
            return Err(SeamError::Refused(
                ucdk_release_proto::v1::Reason::SetMember,
            ));
        }
        if self.depth > 0
            && self
                .files
                .keys()
                .any(|held| clashes_within(self.depth, rel, held))
        {
            return Err(SeamError::Refused(
                ucdk_release_proto::v1::Reason::SetMember,
            ));
        }
        self.files.insert(rel.to_owned(), bytes.to_vec());
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
        Ok(Box::new(Writer {
            files: self.entries.entry(key.clone()).or_default(),
            depth: self.depth,
        }))
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
