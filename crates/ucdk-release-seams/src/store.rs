// SPDX-License-Identifier: Apache-2.0
//! The keyed tree a set is written into. A member is a file or an OCI layout directory, so the
//! store holds a tree under each key.

use ucdk_release::store::StoreKey;

use crate::SeamError;

/// Scoped to one key: a build system handed one can write nothing outside its own entry.
pub trait EntryWriter {
    /// `rel` is relative with no `..` part, no empty part, no backslash, no control character and
    /// never absolute, or the call is `Refused`; a refused path changes nothing and does not
    /// spoil the writer. Any other path is written, and the bar is on a `..` part alone: `a..b`,
    /// `..a`, `...`, a name with a colon, a space, a percent sign or a quote in it is a path like
    /// any other, and no character of it is decoded, rewritten or dropped.
    ///
    /// The paths of an entry form a tree, as the output of a build does, so a file and a
    /// directory of one name cannot both be in an entry: `put_file("a/b")` when `a` was written
    /// as a file, and `put_file("a")` when a path below `a` was written, are `Refused`. A path
    /// that was written holds the bytes of its last write, however often it was written.
    ///
    /// Names are byte-exact: no case is folded and no Unicode composition is normalized, so two
    /// paths that differ in case, in a trailing space, in a trailing dot or in the composition of
    /// a character, in the file name or in a directory, are two. A store whose file system folds
    /// or normalizes names must encode its paths so that this holds.
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError>;
}

pub trait ArtifactStore: Send {
    /// An empty entry; `Refused` when it exists.
    fn begin(&mut self, key: &StoreKey) -> Result<Box<dyn EntryWriter + '_>, SeamError>;

    /// Every relative path of the entry, each a file, sorted bytewise (the order of the bytes of
    /// the UTF-8 text, so a character above U+FFFF comes after U+FF5E and `f10` before `f2`);
    /// empty when there is no entry. No page limit applies: an entry of any number of paths is
    /// listed whole.
    fn files(&mut self, key: &StoreKey) -> Result<Vec<String>, SeamError>;

    fn get(&mut self, key: &StoreKey, rel: &str) -> Result<Option<Vec<u8>>, SeamError>;

    /// The whole entry.
    fn discard(&mut self, key: &StoreKey) -> Result<(), SeamError>;

    /// Every key, for the check that nothing else is in the store; no page limit applies.
    fn keys(&mut self) -> Result<Vec<StoreKey>, SeamError>;
}
