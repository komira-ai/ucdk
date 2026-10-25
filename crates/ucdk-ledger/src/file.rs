// SPDX-License-Identifier: Apache-2.0
//! The file ledger: the document in one file, replaced atomically on every write.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::doc::Doc;
use crate::format::{self, FormatError, MAJOR, MINOR};
use crate::lease::{Clock, Fence, Lease, LeaseError, SystemClock};
use crate::record::{Change, Ledger, LedgerError, Snapshot, Token};

/// Why a file ledger was not opened. Nothing is written in any of these cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// Another handle (in this process or another) has the ledger open.
    Locked { path: PathBuf },
    /// The file is not a whole, valid ledger: it was cut short, changed, or is not a ledger.
    /// It is left as it is; nothing is repaired.
    Corrupt {
        path: PathBuf,
        line: usize,
        why: String,
    },
    /// The file was written by a newer major version of the format than this build reads.
    NewerMajor { path: PathBuf, found: u32 },
    /// The file or its directory could not be read or locked.
    Io { path: PathBuf, why: String },
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked { path } => write!(
                f,
                "{}: the ledger is open elsewhere (one writer at a time)",
                path.display()
            ),
            Self::Corrupt { path, line, why } => write!(
                f,
                "{}: the ledger is corrupt at line {line}: {why}; it was not changed",
                path.display()
            ),
            Self::NewerMajor { path, found } => write!(
                f,
                "{}: the ledger is format version {found}, newer than this build reads \
                 (version {MAJOR}); use a newer build",
                path.display()
            ),
            Self::Io { path, why } => write!(f, "{}: {why}", path.display()),
        }
    }
}

impl std::error::Error for OpenError {}

/// What the handle holds of the file: the document as of the last read or write.
#[derive(Debug)]
struct Document {
    /// The format minor the file was written with. A build refuses to rewrite a file with a
    /// newer minor than its own, because it would drop the tags it does not know.
    minor: u32,
    doc: Doc,
}

#[derive(Debug)]
struct Inner {
    path: PathBuf,
    document: Mutex<Document>,
    clock: Arc<dyn Clock>,
    /// Holds the advisory lock on `<path>.lock` until the last handle is dropped.
    _lock: File,
}

/// A ledger kept in a file. For one machine and one writer: it lets a run resume after a crash
/// of the process, and is not a team backend.
///
/// Opening takes an advisory lock, so a second open of the same path (here or in another
/// process) is refused while the first handle, or a clone of it, is alive. The lock is not a
/// lease: it does not expire, and it does not stop a process that ignores it. The lease is in the
/// document (see [`crate::lease`]): it expires, and a write under a fence that is not the
/// current one is refused, so a writer that lost the lease cannot write whatever it holds. A
/// lease change is a write of the file too, but does not move the version. A write is
/// whole-document: the new document is written to a file in the same directory and flushed,
/// that file replaces the ledger by rename, and the directory is flushed, so a reader sees the
/// old document or the new one and never a mixture. The file is readable by its owner only.
#[derive(Debug, Clone)]
pub struct FileLedger {
    inner: Arc<Inner>,
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn private_file() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
}

fn io_error(path: &Path, err: &io::Error) -> OpenError {
    OpenError::Io {
        path: path.to_owned(),
        why: err.to_string(),
    }
}

/// The document in the file at `path`; `None` when there is no file.
fn load(path: &Path) -> Result<Option<Document>, OpenError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(path, &e)),
    };
    match format::decode(&bytes) {
        Ok(parsed) => Ok(Some(Document {
            minor: parsed.minor,
            doc: parsed.doc,
        })),
        Err(FormatError::Corrupt { line, why }) => Err(OpenError::Corrupt {
            path: path.to_owned(),
            line,
            why,
        }),
        Err(FormatError::NewerMajor { found }) => Err(OpenError::NewerMajor {
            path: path.to_owned(),
            found,
        }),
    }
}

impl FileLedger {
    /// Opens the ledger at `path`, which need not exist yet (it is made by the first write; its
    /// directory must exist). Takes the advisory lock on `<path>.lock`, and removes a
    /// `<path>.tmp` left by a write that never finished.
    ///
    /// # Errors
    ///
    /// [`OpenError`] when the ledger is open elsewhere, the file is corrupt or newer than this
    /// build reads, or the file system refuses.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OpenError> {
        Self::open_with_clock(path, Arc::new(SystemClock))
    }

    /// [`FileLedger::open`], with leases timed by `clock`.
    ///
    /// # Errors
    ///
    /// As [`FileLedger::open`].
    pub fn open_with_clock(
        path: impl AsRef<Path>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, OpenError> {
        let path = path.as_ref().to_owned();
        let lock_path = sibling(&path, ".lock");
        let lock = private_file()
            .read(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| io_error(&lock_path, &e))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(OpenError::Locked { path }),
            Err(TryLockError::Error(e)) => return Err(io_error(&lock_path, &e)),
        }
        let stale = sibling(&path, ".tmp");
        match fs::remove_file(&stale) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(&stale, &e)),
        }
        let document = load(&path)?.unwrap_or_else(|| Document {
            minor: MINOR,
            doc: Doc::default(),
        });
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                document: Mutex::new(document),
                clock,
                _lock: lock,
            }),
        })
    }

    /// Replaces the file with `text`: temp file, flush, rename, flush the directory.
    fn store(&self, text: &str) -> io::Result<()> {
        let path = &self.inner.path;
        let tmp = sibling(path, ".tmp");
        let mut file = private_file().truncate(true).open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)?;
        let dir = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        File::open(dir)?.sync_all()
    }
}

impl FileLedger {
    fn document(&self) -> MutexGuard<'_, Document> {
        self.inner
            .document
            .lock()
            .expect("the lock is never poisoned")
    }

    /// Stores `next` as the document, if the file can be rewritten by this build, and keeps it
    /// in `document` once it is stored. On failure the write may or may not have landed: take
    /// the file as the truth, when there is one and it can be read, so the next write is
    /// compared against what is there.
    fn persist(&self, document: &mut Document, next: Doc) -> Result<(), String> {
        if document.minor > MINOR {
            return Err(format!(
                "{}: written by a newer build (format {MAJOR}.{}, this build writes {MAJOR}.{MINOR}); \
                 rewriting it would drop what this build does not know",
                self.inner.path.display(),
                document.minor
            ));
        }
        match self.store(&format::encode(&next)) {
            Ok(()) => {
                document.doc = next;
                Ok(())
            }
            Err(e) => {
                if let Ok(Some(disk)) = load(&self.inner.path) {
                    *document = disk;
                }
                Err(format!(
                    "{}: the write failed ({e}); it may or may not have landed",
                    self.inner.path.display()
                ))
            }
        }
    }

    fn put(
        &self,
        fence: Option<Fence>,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError> {
        let mut document = self.document();
        document.doc.admit(self.inner.clock.now_millis(), fence)?;
        if expected.0 != document.doc.version {
            return Err(LedgerError::Stale);
        }
        let mut next = document.doc.clone();
        let version = next.apply(change);
        self.persist(&mut document, next)
            .map_err(LedgerError::Unavailable)?;
        Ok(Token(version))
    }
}

impl Ledger for FileLedger {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        let document = self.document();
        Ok(Snapshot {
            token: Token(document.doc.version),
            records: document.doc.records.clone(),
        })
    }

    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError> {
        self.put(None, expected, change)
    }

    fn write_fenced(
        &self,
        fence: Fence,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError> {
        self.put(Some(fence), expected, change)
    }

    fn acquire(&self, holder: &str, ttl: Duration) -> Result<Lease, LeaseError> {
        let mut document = self.document();
        let mut next = document.doc.clone();
        let lease = next.acquire(self.inner.clock.now_millis(), holder, ttl)?;
        self.persist(&mut document, next)
            .map_err(LeaseError::Unavailable)?;
        Ok(lease)
    }

    fn renew(&self, fence: Fence, ttl: Duration) -> Result<Lease, LedgerError> {
        let mut document = self.document();
        let mut next = document.doc.clone();
        let lease = next.renew(self.inner.clock.now_millis(), fence, ttl)?;
        self.persist(&mut document, next)
            .map_err(LedgerError::Unavailable)?;
        Ok(lease)
    }

    fn release(&self, fence: Fence) -> Result<(), LedgerError> {
        let mut document = self.document();
        let mut next = document.doc.clone();
        next.release(fence)?;
        self.persist(&mut document, next)
            .map_err(LedgerError::Unavailable)
    }
}
