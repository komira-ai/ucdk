// SPDX-License-Identifier: Apache-2.0
//! A tree of regular files, directories and symlinks in memory, which the build systems read
//! through. The tree is flat: a node is found by its absolute path, and a symlink has nothing
//! below it that a listing shows.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_release_seams::{DirEntry, FileKind, OutputFiles, SeamError, check_path};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    File(Vec<u8>),
    Directory,
    Symlink,
}

#[derive(Debug, Default)]
pub struct FakeOutputFiles {
    tree: BTreeMap<String, Node>,
    missing: BTreeSet<String>,
    unreadable: BTreeSet<String>,
    /// A file that has other bytes once the directory above it has been listed.
    after_listing: BTreeMap<String, Vec<u8>>,
    calls: Vec<String>,
}

impl FakeOutputFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// A regular file at `path`, with its directories.
    pub fn file(&mut self, path: &str, bytes: &[u8]) {
        self.parents(path);
        self.tree
            .insert(path.to_owned(), Node::File(bytes.to_vec()));
    }

    /// A directory at `path`, with its parents.
    pub fn dir(&mut self, path: &str) {
        self.parents(path);
        self.tree.insert(path.to_owned(), Node::Directory);
    }

    /// A symlink at `path`, to a file, a directory or nothing.
    pub fn symlink(&mut self, path: &str) {
        self.parents(path);
        self.tree.insert(path.to_owned(), Node::Symlink);
    }

    fn parents(&mut self, path: &str) {
        for (at, _) in path.match_indices('/').filter(|(at, _)| *at > 0) {
            self.tree
                .entry(path[..at].to_owned())
                .or_insert(Node::Directory);
        }
    }

    // ---- faults ----

    /// `stat` answers `None` at `path`, and `list` and `read` answer `Permanent`. The listing of
    /// its parent does not name it, and nothing below it exists either.
    pub fn missing(&mut self, path: &str) {
        self.missing.insert(path.to_owned());
    }

    /// A `stat`, `list` or `read` of `path` fails with `Unknown`. The listing of its parent
    /// does not read the entry, so it still names it.
    pub fn unreadable(&mut self, path: &str) {
        self.unreadable.insert(path.to_owned());
    }

    /// The file at `path` is a symlink from now on.
    pub fn symlink_at(&mut self, path: &str) {
        self.symlink(path);
    }

    /// The file at `path` holds `bytes` once any directory above it has been listed.
    pub fn changes_after_listing(&mut self, path: &str, bytes: &[u8]) {
        self.after_listing.insert(path.to_owned(), bytes.to_vec());
    }

    /// Every call, by method and path, in order.
    pub fn calls(&self) -> &[String] {
        &self.calls
    }

    /// The paths and what is at them, to compare before and after a call.
    pub fn snapshot(&self) -> Vec<(String, FileKind, usize)> {
        self.tree
            .iter()
            .map(|(path, node)| match node {
                Node::File(bytes) => (path.clone(), FileKind::File, bytes.len()),
                Node::Directory => (path.clone(), FileKind::Directory, 0),
                Node::Symlink => (path.clone(), FileKind::Symlink, 0),
            })
            .collect()
    }

    /// The common start of every call: log it, check the path, fail an unreadable path.
    fn enter(&mut self, method: &str, path: &str) -> Result<(), SeamError> {
        self.calls.push(format!("{method} {path}"));
        check_path(path)?;
        if self.unreadable.contains(path) {
            return Err(SeamError::Unknown(format!("{path} cannot be read")));
        }
        Ok(())
    }

    /// The node at `path`, unless the path or a directory above it was made missing.
    fn node(&self, path: &str) -> Option<&Node> {
        let gone = self.missing.iter().any(|m| {
            path == m
                || path
                    .strip_prefix(m.as_str())
                    .is_some_and(|r| r.starts_with('/'))
        });
        self.tree.get(path).filter(|_| !gone)
    }
}

fn kind_of(node: &Node) -> FileKind {
    match node {
        Node::File(_) => FileKind::File,
        Node::Directory => FileKind::Directory,
        Node::Symlink => FileKind::Symlink,
    }
}

impl OutputFiles for FakeOutputFiles {
    fn stat(&mut self, path: &str) -> Result<Option<FileKind>, SeamError> {
        self.enter("stat", path)?;
        Ok(self.node(path).map(kind_of))
    }

    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, SeamError> {
        self.enter("list", path)?;
        if self.node(path) != Some(&Node::Directory) {
            return Err(SeamError::Permanent(format!("{path} is not a directory")));
        }
        let below = format!("{}/", path.trim_end_matches('/'));
        let mut entries = Vec::new();
        let mut linked: Vec<String> = Vec::new();
        // The paths come out in bytewise order, which is the order of `rel` under one prefix.
        for (full, node) in self.tree.range(below.clone()..) {
            let Some(rel) = full.strip_prefix(&below) else {
                break;
            };
            // What stat says is absent is not listed.
            if self.node(full).is_none() {
                continue;
            }
            // A symlink is an entry and is never descended: what the flat tree holds under one
            // is not listed.
            if linked.iter().any(|link| rel.starts_with(link)) {
                continue;
            }
            if *node == Node::Symlink {
                linked.push(format!("{rel}/"));
            }
            entries.push(DirEntry {
                rel: rel.to_owned(),
                kind: kind_of(node),
            });
        }
        // What was to change once this directory was listed changes now, and only once.
        let tree = &mut self.tree;
        self.after_listing.retain(|path, bytes| {
            let changing = path.starts_with(&below);
            if changing {
                tree.insert(path.clone(), Node::File(std::mem::take(bytes)));
            }
            !changing
        });
        Ok(entries)
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, SeamError> {
        self.enter("read", path)?;
        match self.node(path) {
            Some(Node::File(bytes)) => Ok(bytes.clone()),
            _ => Err(SeamError::Permanent(format!(
                "{path} is not a regular file"
            ))),
        }
    }
}
