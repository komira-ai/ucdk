// SPDX-License-Identifier: Apache-2.0
//! The local disk, read through the `OutputFiles` seam.

use std::fs::{self, FileType};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{DirEntry, FileKind, OutputFiles, SeamError, check_path};

/// An [`OutputFiles`] over the local disk, confined to the roots it is made with.
///
/// A path is made of what a build system answered, so it is held to the contract before it is
/// looked up: it must be absolute and have no `..` part, and its `.` parts and empty parts
/// (a repeated slash, a trailing one) are dropped here, so that a slashed path answers as the
/// unslashed one. It must also be a root or below one (`Refused`, as a path with a `..` part
/// is), and no part of it between the root and its last part may be a symlink, wherever the
/// link leads: a path is never resolved through a link, so an output cannot reach out of its
/// root by one. The last part of a path is never followed when it is a symlink: `stat` says
/// `Symlink`, and `list` and `read` refuse it. Nothing here writes.
///
/// The check and the read are two calls, so a link put in place between them is not seen; the
/// tree is the build system's output, which nothing else writes while it is read.
#[derive(Debug, Clone)]
pub struct LocalFiles {
    roots: Vec<Root>,
}

/// A directory the reader may read below: as it was named, and as it is on the disk.
#[derive(Debug, Clone)]
struct Root {
    named: PathBuf,
    real: PathBuf,
}

impl Root {
    /// The root as the disk has it and the part of `path` below the root, when `path` is the
    /// root or below it, under the name it was given or under its real one.
    fn below<'a>(&'a self, path: &'a Path) -> Option<(&'a Path, &'a Path)> {
        [&self.named, &self.real]
            .into_iter()
            .find_map(|base| path.strip_prefix(base).ok())
            .map(|rel| (self.real.as_path(), rel))
    }
}

impl LocalFiles {
    /// A reader that reads below `roots` and nowhere else: the output root the build system
    /// reports (the execution root of Bazel; the project root or `buck-out` of Buck2) and the
    /// directory of the report files. A root is resolved when the reader is made, and a link on
    /// the way to it is that way; only the parts below it are held to the rule about links.
    ///
    /// # Errors
    ///
    /// `Permanent` when a root cannot be resolved.
    pub fn within<P: AsRef<Path>>(roots: impl IntoIterator<Item = P>) -> Result<Self, SeamError> {
        let roots = roots
            .into_iter()
            .map(|named| {
                let named = named.as_ref().to_path_buf();
                fs::canonicalize(&named)
                    .map(|real| Root {
                        named: named.clone(),
                        real,
                    })
                    .map_err(|error| {
                        SeamError::Permanent(format!(
                            "{} could not be resolved: {error}",
                            named.display()
                        ))
                    })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { roots })
    }

    /// `Refused` unless `path` is a root or below one with no symlink between the root and its
    /// last part. A part that is not there, or is not a directory, ends the walk: what is below
    /// it is nothing, which the lookup says.
    fn confine(&self, path: &Path) -> Result<(), SeamError> {
        let Some((real, rel)) = self.roots.iter().find_map(|root| root.below(path)) else {
            return Err(SeamError::Refused(Reason::BuildOutput));
        };
        let parts: Vec<_> = rel.components().collect();
        let mut at = real.to_path_buf();
        for part in parts.iter().take(parts.len().saturating_sub(1)) {
            at.push(part);
            match fs::symlink_metadata(&at) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(SeamError::Refused(Reason::BuildOutput));
                }
                Ok(_) => {}
                Err(error) if is_absent(&error) => return Ok(()),
                Err(error) => return Err(unknown("examined", &at, &error)),
            }
        }
        Ok(())
    }

    /// The path to look at, and what is there: `None` when nothing is, which is a path that is
    /// missing and a path below something that is not a directory. A failure to ask is
    /// `Unknown`, and a node of no kind that a build system knows is `Permanent`.
    fn probe(&self, given: &str) -> Result<(PathBuf, Option<FileKind>), SeamError> {
        let path = normal(given)?;
        self.confine(&path)?;
        match fs::symlink_metadata(&path) {
            Ok(meta) => match kind_of(meta.file_type()) {
                Some(kind) => Ok((path, Some(kind))),
                None => Err(SeamError::Permanent(format!(
                    "{given} is not a file, a directory or a symlink"
                ))),
            },
            Err(error) if is_absent(&error) => Ok((path, None)),
            Err(error) => Err(unknown("examined", &path, &error)),
        }
    }
}

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

/// Whether the failure to examine a path says that nothing is there: the path is missing, or
/// something above it is not a directory.
fn is_absent(error: &std::io::Error) -> bool {
    matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory)
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
        self.probe(path).map(|(_, kind)| kind)
    }

    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, SeamError> {
        let (dir, kind) = self.probe(path)?;
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
        let (file, kind) = self.probe(path)?;
        if kind != Some(FileKind::File) {
            return Err(SeamError::Permanent(format!(
                "{path} is not a regular file that exists"
            )));
        }
        fs::read(&file).map_err(|error| unknown("read", &file, &error))
    }
}
