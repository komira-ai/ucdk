// SPDX-License-Identifier: Apache-2.0
//! The local disk, read through the `OutputFiles` seam.

use std::fs::{self, FileType};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use ucdk_release_seams::{DirEntry, FileKind, OutputFiles, SeamError, check_path};

/// An [`OutputFiles`] over the local disk.
///
/// A path is made of what a build system answered, so it is held to the contract before it is
/// looked up: it must be absolute and have no `..` part, and its `.` parts and empty parts
/// (a repeated slash, a trailing one) are dropped here, so that a slashed path answers as the
/// unslashed one. The last part of a path is never followed when it is a symlink: `stat` says
/// `Symlink`, and `list` and `read` refuse it. Nothing here writes.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalFiles;

/// The path `given` names, without its `.` parts and empty parts; `Refused` when it is not an
/// absolute path without a `..` part.
fn normal(given: &str) -> Result<PathBuf, SeamError> {
    check_path(given)?;
    let parts: Vec<&str> = given
        .split('/')
        .filter(|part| !matches!(*part, "" | "."))
        .collect();
    Ok(PathBuf::from(format!("/{}", parts.join("/"))))
}

/// What a file type is, when it is one of the three kinds; a socket, a pipe and a device are
/// none of them.
fn kind_of(file_type: FileType) -> Option<FileKind> {
    if file_type.is_symlink() {
        Some(FileKind::Symlink)
    } else if file_type.is_dir() {
        Some(FileKind::Directory)
    } else if file_type.is_file() {
        Some(FileKind::File)
    } else {
        None
    }
}

fn unknown(what: &str, path: &Path, error: &std::io::Error) -> SeamError {
    SeamError::Unknown(format!("{} could not be {what}: {error}", path.display()))
}

/// The path to look at, and what is there: `None` when nothing is, which is a path that is
/// missing and a path below something that is not a directory. A failure to ask is `Unknown`,
/// and a node of no kind that a build system knows is `Permanent`.
fn probe(given: &str) -> Result<(PathBuf, Option<FileKind>), SeamError> {
    let path = normal(given)?;
    match fs::symlink_metadata(&path) {
        Ok(meta) => match kind_of(meta.file_type()) {
            Some(kind) => Ok((path, Some(kind))),
            None => Err(SeamError::Permanent(format!(
                "{given} is not a file, a directory or a symlink"
            ))),
        },
        Err(error) if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Ok((path, None))
        }
        Err(error) => Err(unknown("examined", &path, &error)),
    }
}

/// Every entry below `dir`, named relative to the directory listed (`rel` is the part from
/// there to `dir`, empty at the top).
fn walk(dir: &Path, rel: &str, into: &mut Vec<DirEntry>) -> Result<(), SeamError> {
    // One failure for the directory and for any entry of it, so that no entry is skipped.
    let entries = fs::read_dir(dir)
        .and_then(|entries| {
            entries
                .map(|entry| {
                    entry.and_then(|entry| entry.file_type().map(|kind| (entry.file_name(), kind)))
                })
                .collect::<std::io::Result<Vec<_>>>()
        })
        .map_err(|error| unknown("listed", dir, &error))?;
    for (name, file_type) in entries {
        let name = name.into_string().map_err(|name| {
            SeamError::Permanent(format!(
                "{} holds the name {name:?}, which is not text",
                dir.display()
            ))
        })?;
        let kind = kind_of(file_type).ok_or_else(|| {
            SeamError::Permanent(format!(
                "{}/{name} is not a file, a directory or a symlink",
                dir.display()
            ))
        })?;
        let below = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        into.push(DirEntry {
            rel: below.clone(),
            kind,
        });
        // A symlink is an entry and is not descended.
        if kind == FileKind::Directory {
            walk(&dir.join(&name), &below, into)?;
        }
    }
    Ok(())
}

impl OutputFiles for LocalFiles {
    fn stat(&mut self, path: &str) -> Result<Option<FileKind>, SeamError> {
        probe(path).map(|(_, kind)| kind)
    }

    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, SeamError> {
        let (dir, kind) = probe(path)?;
        if kind != Some(FileKind::Directory) {
            return Err(SeamError::Permanent(format!(
                "{path} is not a directory that exists"
            )));
        }
        let mut entries = Vec::new();
        walk(&dir, "", &mut entries)?;
        // Bytewise by the whole of `rel`, which is not the order of a walk: `a-b` is before `a/b`.
        entries.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(entries)
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, SeamError> {
        let (file, kind) = probe(path)?;
        if kind != Some(FileKind::File) {
            return Err(SeamError::Permanent(format!(
                "{path} is not a regular file that exists"
            )));
        }
        fs::read(&file).map_err(|error| unknown("read", &file, &error))
    }
}
