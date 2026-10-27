// SPDX-License-Identifier: Apache-2.0
//! A read-only view of local files: the build system's output tree and the report file of a
//! test invocation. It has no write method, so a build system cannot change the tree.

use ucdk_release_proto::v1::Reason;

use crate::SeamError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    File,
    Directory,
    Symlink,
}

/// One entry below a listed directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Relative to the listed directory, `/` separated.
    pub rel: String,
    pub kind: FileKind,
}

/// Every `path` is absolute with no `..` part (see [`check_path`]) and otherwise `Refused`. Only
/// the last component of a path is looked at without following, so a symlink is never followed.
pub trait OutputFiles: Send {
    /// What is at `path`; `Ok(None)` is "nothing there", the only answer that tells absence
    /// from a failure (`Unknown`).
    fn stat(&mut self, path: &str) -> Result<Option<FileKind>, SeamError>;

    /// Every entry below the directory `path`, recursively, sorted bytewise by `rel`. A missing
    /// path, or one that is not a directory, is `Permanent` and never an empty listing.
    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, SeamError>;

    /// The bytes of the regular file at `path`, as they are at the call. A symlink, a directory
    /// or nothing is `Permanent`.
    fn read(&mut self, path: &str) -> Result<Vec<u8>, SeamError>;
}

/// A relative path of an entry: non-empty, no empty part, no `.` or `..` part, not absolute.
pub fn valid_rel(rel: &str) -> bool {
    rel.split('/')
        .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// A path a reader may be asked about: absolute, with no `..` part.
pub fn check_path(path: &str) -> Result<(), SeamError> {
    if path.starts_with('/') && !path.split('/').any(|part| part == "..") {
        Ok(())
    } else {
        Err(SeamError::Refused(Reason::BuildOutput))
    }
}

/// The path of the listed entry `rel` of the directory `dir`; `Refused` when `rel` is not a
/// relative path of an entry. It is the one place the rule about `rel` is written.
pub fn entry_path(dir: &str, rel: &str) -> Result<String, SeamError> {
    if valid_rel(rel) {
        Ok(format!("{}/{rel}", dir.trim_end_matches('/')))
    } else {
        Err(SeamError::Refused(Reason::BuildOutput))
    }
}
