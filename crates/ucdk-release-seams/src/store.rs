// SPDX-License-Identifier: Apache-2.0
//! The keyed tree a set is written into. A member is a file or an OCI layout directory, so the
//! store holds a tree under each key.

use ucdk_release::store::StoreKey;

use crate::SeamError;

/// Scoped to one key: a build system handed one can write nothing outside its own entry.
pub trait EntryWriter {
    /// `rel` is relative with no `..`, no empty part and never absolute, or the call is
    /// `Refused`.
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError>;
}

pub trait ArtifactStore: Send {
    /// An empty entry; `Refused` when it exists.
    fn begin(&mut self, key: &StoreKey) -> Result<Box<dyn EntryWriter + '_>, SeamError>;

    /// Every relative path of the entry, sorted; empty when there is no entry.
    fn files(&mut self, key: &StoreKey) -> Result<Vec<String>, SeamError>;

    fn get(&mut self, key: &StoreKey, rel: &str) -> Result<Option<Vec<u8>>, SeamError>;

    /// The whole entry.
    fn discard(&mut self, key: &StoreKey) -> Result<(), SeamError>;

    /// Every key, for the check that nothing else is in the store.
    fn keys(&mut self) -> Result<Vec<StoreKey>, SeamError>;
}
