// SPDX-License-Identifier: Apache-2.0
//! The `OutputFiles` rig: the fake tree, and the stubs that break one promise of the reader.

use std::collections::BTreeMap;

use ucdk_release_fake::FakeOutputFiles;
use ucdk_release_seams::kit::{Node, OutputFilesRig};
use ucdk_release_seams::{DirEntry, FileKind, OutputFiles, SeamError};

pub const ROOT: &str = "/work";

/// What a stub gets wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// A listing in directory-walk order: a directory's children right after it.
    WalkOrder,
    /// A listing that calls every entry a file.
    EveryEntryIsAFile,
    /// A listing that follows a symlink to a directory.
    FollowsLinks,
    /// A `stat` that follows a symlink.
    StatFollows,
    /// A `list` of a path where nothing is that answers an empty listing.
    ListOfMissingIsEmpty,
    /// A `read` that follows a symlink.
    ReadFollowsLink,
    /// A `read` of a file directly below the root that needs a directory listed above it.
    NeedsListedDirAtTheRoot,
    /// A `read` of a file deeper than that which needs a directory listed above it.
    NeedsListedDirBelowTheRoot,
    /// A `read` of a directory that has just been `stat`ed that answers an empty file.
    ReadOfAStattedDirectoryIsEmpty,
    /// A `read` that resolves a path with `..` parts instead of refusing it.
    ReadResolvesClimbs,
    /// A `read` of a file nested below a listed directory that answers `Permanent`: the entry
    /// path of the file drops the part between.
    NestedEntryIsUnreadable,
    /// A `read` that keeps its first answer.
    CachesReads,
    /// A `read` that touches the file it read.
    ReadTouches,
}

pub struct FakeFilesRig {
    pub flaw: Option<Flaw>,
}

impl FakeFilesRig {
    pub fn conforming() -> Self {
        Self { flaw: None }
    }

    pub fn flawed(flaw: Flaw) -> Self {
        Self { flaw: Some(flaw) }
    }
}

pub struct Flawed {
    inner: FakeOutputFiles,
    flaw: Option<Flaw>,
    listed: Vec<String>,
    last_stat: Option<String>,
    cache: BTreeMap<String, Vec<u8>>,
}

fn full(rel: &str) -> String {
    format!("{ROOT}/{rel}")
}

impl OutputFilesRig for FakeFilesRig {
    type Files = Flawed;

    fn root(&self) -> String {
        ROOT.to_owned()
    }

    fn reader(&self, tree: &[(&str, Node<'_>)]) -> Flawed {
        let mut inner = FakeOutputFiles::new();
        for (rel, node) in tree {
            match node {
                Node::File(bytes) => inner.file(&full(rel), bytes),
                Node::Directory => inner.dir(&full(rel)),
                Node::Link(_) => inner.symlink(&full(rel)),
            }
        }
        Flawed {
            inner,
            flaw: self.flaw,
            listed: Vec::new(),
            last_stat: None,
            cache: BTreeMap::new(),
        }
    }

    fn change(&self, files: &mut Flawed, rel: &str, bytes: &[u8]) {
        files.inner.file(&full(rel), bytes);
    }

    fn snapshot(&self, files: &Flawed) -> Vec<(String, FileKind, usize)> {
        files.inner.snapshot()
    }
}

/// `path` with its `..` parts resolved.
fn resolved(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/').skip(1) {
        if part == ".." {
            parts.pop();
        } else {
            parts.push(part);
        }
    }
    format!("/{}", parts.join("/"))
}

impl Flawed {
    fn is(&self, flaw: Flaw) -> bool {
        self.flaw == Some(flaw)
    }

    /// The listed directory above `path`, if one was listed.
    fn listed_above(&self, path: &str) -> Option<&String> {
        self.listed
            .iter()
            .find(|dir| path.starts_with(&format!("{dir}/")))
    }
}

impl OutputFiles for Flawed {
    fn stat(&mut self, path: &str) -> Result<Option<FileKind>, SeamError> {
        self.last_stat = Some(path.to_owned());
        let kind = self.inner.stat(path)?;
        if self.is(Flaw::StatFollows) && kind == Some(FileKind::Symlink) {
            return Ok(Some(FileKind::File));
        }
        Ok(kind)
    }

    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, SeamError> {
        if self.is(Flaw::ListOfMissingIsEmpty) && self.inner.stat(path) == Ok(None) {
            return Ok(Vec::new());
        }
        let mut entries = self.inner.list(path)?;
        self.listed.push(path.trim_end_matches('/').to_owned());
        if self.is(Flaw::WalkOrder) {
            entries.sort_by_key(|e| e.rel.replace('/', "\u{0}"));
        }
        if self.is(Flaw::EveryEntryIsAFile) {
            for entry in &mut entries {
                entry.kind = FileKind::File;
            }
        }
        if self.is(Flaw::FollowsLinks) {
            let below: Vec<DirEntry> = entries
                .iter()
                .filter_map(|e| {
                    e.rel.strip_prefix("real/").map(|rest| DirEntry {
                        rel: format!("link/{rest}"),
                        kind: e.kind,
                    })
                })
                .collect();
            entries.extend(below);
            entries.sort_by(|a, b| a.rel.cmp(&b.rel));
        }
        Ok(entries)
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, SeamError> {
        if self.is(Flaw::CachesReads)
            && let Some(bytes) = self.cache.get(path)
        {
            return Ok(bytes.clone());
        }
        let mut result = self.inner.read(path);
        match self.flaw {
            Some(Flaw::ReadFollowsLink)
                if result.is_err() && self.inner.stat(path) == Ok(Some(FileKind::Symlink)) =>
            {
                result = Ok(b"target".to_vec());
            }
            Some(Flaw::ReadOfAStattedDirectoryIsEmpty)
                if result.is_err()
                    && self.last_stat.as_deref() == Some(path)
                    && self.inner.stat(path) == Ok(Some(FileKind::Directory)) =>
            {
                result = Ok(Vec::new());
            }
            Some(Flaw::ReadResolvesClimbs) if result.is_err() && path.contains("/../") => {
                result = self.inner.read(&resolved(path));
            }
            Some(Flaw::NeedsListedDirAtTheRoot | Flaw::NeedsListedDirBelowTheRoot)
                if result.is_ok() =>
            {
                let depth = path
                    .strip_prefix(&format!("{ROOT}/"))
                    .map_or(0, |rest| rest.split('/').count());
                let wrong_depth = if self.is(Flaw::NeedsListedDirAtTheRoot) {
                    depth == 1
                } else {
                    depth > 1
                };
                if wrong_depth && self.listed_above(path).is_none() {
                    result = Err(SeamError::Permanent(format!("{path} was not listed")));
                }
            }
            Some(Flaw::NestedEntryIsUnreadable) if result.is_ok() => {
                let nested = self
                    .listed_above(path)
                    .is_some_and(|dir| path[dir.len() + 1..].contains('/'));
                if nested {
                    result = Err(SeamError::Permanent(format!("{path} is not an entry")));
                }
            }
            _ => {}
        }
        if let Ok(bytes) = &result {
            if self.is(Flaw::CachesReads) {
                self.cache.insert(path.to_owned(), bytes.clone());
            }
            if self.is(Flaw::ReadTouches) {
                let mut touched = bytes.clone();
                touched.push(b'!');
                self.inner.file(path, &touched);
            }
        }
        result
    }
}
