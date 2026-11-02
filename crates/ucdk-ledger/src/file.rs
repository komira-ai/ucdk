// SPDX-License-Identifier: Apache-2.0
//! The file ledger: the document in one file, replaced atomically on every write.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
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
    /// The file name ends in a suffix the file ledger keeps for the temporary and lock files
    /// of another ledger in the same directory (`.tmp` and `.lock`).
    /// Nothing is touched.
    ReservedName { path: PathBuf },
}

/// The suffixes of the files a ledger at `<path>` keeps beside it: `<path>.tmp` and
/// `<path>.lock`. A ledger whose own name ends in one could be another ledger's temporary or
/// lock file, so [`FileLedger::open`] refuses it.
const RESERVED_SUFFIXES: [&str; 2] = [".tmp", ".lock"];

/// Whether the last component of `path` ends in one of `RESERVED_SUFFIXES`.
fn is_reserved(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        let name = name.to_string_lossy();
        RESERVED_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
    })
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
            Self::ReservedName { path } => write!(
                f,
                "{}: a ledger file name may not end in {} or {}, which a ledger keeps for its \
                 temporary and lock files; nothing was changed",
                path.display(),
                RESERVED_SUFFIXES[0],
                RESERVED_SUFFIXES[1]
            ),
        }
    }
}

impl std::error::Error for OpenError {}

/// The document as one read of the file saw it.
#[derive(Debug)]
struct Document {
    /// The format minor the file was written with. A build refuses to rewrite a file with a
    /// newer minor than its own, because it would drop the tags it does not know.
    minor: u32,
    doc: Doc,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            minor: MINOR,
            doc: Doc::default(),
        }
    }
}

#[derive(Debug)]
struct Inner {
    path: PathBuf,
    clock: Arc<dyn Clock>,
    /// `<path>.lock`: held (exclusively) only for the length of one read-modify-write, never
    /// between calls. `None` for a ledger opened read-only, which has no lock file.
    lock: Option<File>,
    /// Orders the calls of the handles that share this one. An advisory lock held through one
    /// open file does not exclude another user of the same file, so threads queue here first.
    turn: Mutex<()>,
}

/// A ledger kept in a file. For one machine: it lets a run resume after a crash of the process,
/// and is not a team backend.
///
/// Any number of handles, in this process or in others, may be open on one path at once. Each
/// call reads the file afresh, and each change takes an exclusive advisory lock on
/// `<path>.lock` for the read-modify-write alone and gives it back before returning, so two
/// changes never interleave. The lock is not a lease: it is never held between calls, so it
/// never keeps a live process out. What arbitrates between two live runs is the lease in the
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
    /// directory must exist). Reads it once to refuse a file that is not a whole ledger, and
    /// removes a `<path>.tmp` left by a write that never finished. Another handle may be open
    /// on the same path. A file name that ends in `.tmp` or `.lock` is refused, because it
    /// could be the temporary or lock file of another ledger in the directory.
    ///
    /// # Errors
    ///
    /// [`OpenError`] when the name is reserved, the file is corrupt or newer than this build
    /// reads, or the file system refuses.
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
        if is_reserved(&path) {
            return Err(OpenError::ReservedName { path });
        }
        let lock_path = sibling(&path, ".lock");
        let lock = private_file()
            .read(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| io_error(&lock_path, &e))?;
        // Nobody else holds this handle yet, so it needs no turn: only the file lock.
        let held = Held::take(&lock).map_err(|e| io_error(&lock_path, &e))?;
        // Writes happen under the lock, so a `.tmp` seen under it is a write that never finished.
        let stale = sibling(&path, ".tmp");
        match fs::remove_file(&stale) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(&stale, &e)),
        }
        load(&path)?;
        drop(held);
        Ok(Self::with_lock(path, clock, Some(lock)))
    }

    /// Opens the ledger at `path` to read it and nothing else: [`FileLedger::open_read_only_with_clock`]
    /// with the wall clock.
    ///
    /// # Errors
    ///
    /// As [`FileLedger::open_read_only_with_clock`].
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, OpenError> {
        Self::open_read_only_with_clock(path, Arc::new(SystemClock))
    }

    /// Opens the ledger at `path` to read it and nothing else. It reads the document once, to
    /// refuse a file that is not a whole ledger, and the file must exist. It creates no
    /// `<path>.lock`, takes no lock, and removes no `<path>.tmp`, so it works in a directory
    /// it may not write and contends with no writer: a write replaces the file whole, so a
    /// read sees the old document or the new one. Every write, renew and release answers
    /// [`LedgerError::ReadOnly`] and every acquire [`LeaseError::ReadOnly`]. A read reports
    /// [`Snapshot::at`] by `clock`, so a caller that fixes its clock sees leases by that clock.
    ///
    /// # Errors
    ///
    /// [`OpenError`] when the file is missing, corrupt, newer than this build reads, or cannot
    /// be read.
    pub fn open_read_only_with_clock(
        path: impl AsRef<Path>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, OpenError> {
        let path = path.as_ref().to_owned();
        if load(&path)?.is_none() {
            return Err(OpenError::Io {
                path,
                why: "the file does not exist".to_owned(),
            });
        }
        Ok(Self::with_lock(path, clock, None))
    }

    fn with_lock(path: PathBuf, clock: Arc<dyn Clock>, lock: Option<File>) -> Self {
        Self {
            inner: Arc::new(Inner {
                path,
                clock,
                lock,
                turn: Mutex::new(()),
            }),
        }
    }

    fn turn(&self) -> MutexGuard<'_, ()> {
        self.inner.turn.lock().expect("the lock is never poisoned")
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

    /// The document in the file now, or why it cannot be had.
    fn current(&self) -> Result<Document, String> {
        load(&self.inner.path)
            .map(Option::unwrap_or_default)
            .map_err(|e| e.to_string())
    }

    /// Begins a read-modify-write: waits for its turn and for the file lock, then reads the
    /// file. Both are given back when the `Txn` is dropped.
    /// A ledger opened read-only has no lock and refuses here, before anything is read.
    fn begin(&self) -> Result<Txn<'_>, Refused> {
        let Some(lock) = &self.inner.lock else {
            return Err(Refused::ReadOnly);
        };
        let turn = self.turn();
        let held = Held::take(lock).map_err(|e| {
            Refused::Unavailable(format!(
                "{}: the lock could not be taken ({e})",
                self.inner.path.display()
            ))
        })?;
        let document = self.current().map_err(Refused::Unavailable)?;
        Ok(Txn {
            ledger: self,
            document,
            _held: held,
            _turn: turn,
        })
    }

    fn change(
        &self,
        fence: Option<Fence>,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError> {
        let txn = self.begin()?;
        let doc = &txn.document.doc;
        doc.admit(self.inner.clock.now_millis(), fence)?;
        if expected.0 != doc.version {
            return Err(LedgerError::Stale);
        }
        let mut next = doc.clone();
        let version = next.apply(change);
        txn.persist(&next).map_err(LedgerError::Unavailable)?;
        Ok(Token(version))
    }
}

/// Why a read-modify-write did not begin.
enum Refused {
    /// The ledger was opened read-only.
    ReadOnly,
    /// The lock or the file could not be had.
    Unavailable(String),
}

impl From<Refused> for LedgerError {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::ReadOnly => Self::ReadOnly,
            Refused::Unavailable(why) => Self::Unavailable(why),
        }
    }
}

impl From<Refused> for LeaseError {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::ReadOnly => Self::ReadOnly,
            Refused::Unavailable(why) => Self::Unavailable(why),
        }
    }
}

/// The exclusive file lock, held until dropped.
struct Held<'a>(&'a File);

impl<'a> Held<'a> {
    fn take(file: &'a File) -> io::Result<Self> {
        file.lock()?;
        Ok(Self(file))
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        // Closing the file gives the lock back too, so a failure here costs nothing.
        let _ = self.0.unlock();
    }
}

/// One read-modify-write, under the file lock.
struct Txn<'a> {
    ledger: &'a FileLedger,
    document: Document,
    // Field order is drop order: the file lock is given back before the turn.
    _held: Held<'a>,
    _turn: MutexGuard<'a, ()>,
}

impl Txn<'_> {
    /// Stores `next` as the document, if the file can be rewritten by this build. On failure
    /// the write may or may not have landed; the next call reads the file and goes by that.
    fn persist(&self, next: &Doc) -> Result<(), String> {
        let path = &self.ledger.inner.path;
        if self.document.minor > MINOR {
            return Err(format!(
                "{}: written by a newer build (format {MAJOR}.{}, this build writes {MAJOR}.{MINOR}); \
                 rewriting it would drop what this build does not know",
                path.display(),
                self.document.minor
            ));
        }
        self.ledger.store(&format::encode(next)).map_err(|e| {
            format!(
                "{}: the write failed ({e}); it may or may not have landed",
                path.display()
            )
        })
    }
}

impl Ledger for FileLedger {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        // A rename replaces the file whole, so a read needs no lock.
        let document = self.current().map_err(LedgerError::Unavailable)?;
        Ok(document.doc.snapshot(self.inner.clock.now_millis()))
    }

    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError> {
        self.change(None, expected, change)
    }

    fn write_fenced(
        &self,
        fence: Fence,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError> {
        self.change(Some(fence), expected, change)
    }

    fn acquire(&self, holder: &str, ttl: Duration) -> Result<Lease, LeaseError> {
        let txn = self.begin()?;
        let mut next = txn.document.doc.clone();
        let lease = next.acquire(self.inner.clock.now_millis(), holder, ttl)?;
        txn.persist(&next).map_err(LeaseError::Unavailable)?;
        Ok(lease)
    }

    fn acquire_if(
        &self,
        holder: &str,
        ttl: Duration,
        token: &Token,
        fence: Fence,
    ) -> Result<Lease, LeaseError> {
        // The document is read inside the lock, so the precondition and the grant are one step.
        let txn = self.begin()?;
        let mut next = txn.document.doc.clone();
        let lease = next.acquire_if(self.inner.clock.now_millis(), holder, ttl, token, fence)?;
        txn.persist(&next).map_err(LeaseError::Unavailable)?;
        Ok(lease)
    }

    fn renew(&self, fence: Fence, ttl: Duration) -> Result<Lease, LedgerError> {
        let txn = self.begin()?;
        let mut next = txn.document.doc.clone();
        let lease = next.renew(self.inner.clock.now_millis(), fence, ttl)?;
        txn.persist(&next).map_err(LedgerError::Unavailable)?;
        Ok(lease)
    }

    fn release(&self, fence: Fence) -> Result<(), LedgerError> {
        let txn = self.begin()?;
        let mut next = txn.document.doc.clone();
        next.release(fence)?;
        txn.persist(&next).map_err(LedgerError::Unavailable)
    }
}
