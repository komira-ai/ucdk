// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code, unsafe_code)]
//! Helpers shared by the ABI tests: the C functions called the way a C caller would, a config for
//! each kind of ledger, a copy of a golden ledger file, and a ledger whose read runs a hook.

use std::path::{Path, PathBuf};
use std::ptr::{self, NonNull};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use prost::Message as _;
use ucdk_api::v1::{Error, SessionConfig};
use ucdk_capi::{Buf, SessionHandle, ucdk_buf_free, ucdk_session_open};
use ucdk_ledger::{
    Change, Fence, Lease, LeaseError, Ledger, LedgerError, MemoryLedger, Snapshot, Token,
};

/// The signature of every verb.
pub type VerbFn = unsafe extern "C" fn(*mut SessionHandle, *const u8, usize, *mut Buf) -> i32;

/// What a call answered: its status and the bytes in `out`, already freed.
#[derive(Debug)]
pub struct Reply {
    pub status: i32,
    pub bytes: Vec<u8>,
}

/// Copies the bytes of a buffer the library filled and frees it.
pub fn take(buf: Buf) -> Vec<u8> {
    if buf.ptr.is_null() {
        assert_eq!(buf.len, 0, "a buffer with no bytes has no length");
        return Vec::new();
    }
    // SAFETY: the library filled `buf` with `len` bytes at `ptr` and nothing has freed it.
    let bytes = unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec();
    // SAFETY: `buf` is as the library gave it out, freed here once.
    unsafe { ucdk_buf_free(buf) };
    bytes
}

/// A buffer that is not empty and is not the library's: a call must overwrite it, never free it.
pub fn junk() -> Buf {
    Buf {
        ptr: NonNull::<u8>::dangling().as_ptr(),
        len: 7,
    }
}

/// Calls a verb the way C would, with `out` starting as junk.
pub fn call(verb: VerbFn, session: *mut SessionHandle, request: &[u8]) -> Reply {
    let mut out = junk();
    // SAFETY: `request` is readable for its length, `out` is a writable `Buf`.
    let status = unsafe { verb(session, request.as_ptr(), request.len(), &raw mut out) };
    Reply {
        status,
        bytes: take(out),
    }
}

/// The opened session, or the status and error bytes of the refusal.
pub struct Opened {
    pub status: i32,
    pub session: *mut SessionHandle,
    pub error: Vec<u8>,
}

pub fn open_bytes(config: &[u8]) -> Opened {
    let mut session = NonNull::<SessionHandle>::dangling().as_ptr();
    let mut error = junk();
    // SAFETY: `config` is readable for its length; both outputs are writable.
    let status = unsafe {
        ucdk_session_open(
            config.as_ptr(),
            config.len(),
            &raw mut session,
            &raw mut error,
        )
    };
    Opened {
        status,
        session,
        error: take(error),
    }
}

pub fn open(config: &SessionConfig) -> Opened {
    open_bytes(&config.encode_to_vec())
}

/// A config for a session on a memory ledger.
pub fn memory_config() -> SessionConfig {
    SessionConfig {
        ledger_kind: "memory".to_owned(),
        ..SessionConfig::default()
    }
}

/// A config for a session on the file ledger at `location`.
pub fn file_config(location: &Path) -> SessionConfig {
    SessionConfig {
        ledger_kind: "file".to_owned(),
        ledger_location: location.to_string_lossy().into_owned(),
        ..SessionConfig::default()
    }
}

/// A session on a memory ledger, opened through the ABI.
pub fn open_memory() -> *mut SessionHandle {
    let opened = open(&memory_config());
    assert_eq!(opened.status, 0);
    assert!(!opened.session.is_null());
    opened.session
}

pub fn error_of(bytes: &[u8]) -> Error {
    Error::decode(bytes).expect("an Error message")
}

pub fn null_session() -> *mut SessionHandle {
    ptr::null_mut()
}

/// A ledger golden of the repository, by file name.
pub fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../sdk/ledger-golden")
        .join(name)
}

/// A directory under the build's scratch area, empty on creation and removed on drop.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("abi-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Self(dir)
    }

    pub fn dir(&self) -> &Path {
        &self.0
    }

    /// The ledger file of the directory.
    pub fn file(&self) -> PathBuf {
        self.0.join("ledger.ucdk")
    }

    /// The directory's files with their bytes, by name.
    pub fn listing(&self) -> std::collections::BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(&self.0)
            .expect("list")
            .map(|entry| {
                let entry = entry.expect("entry");
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path()).expect("read"),
                )
            })
            .collect()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A memory ledger with a hook that runs at the start of every read, a way to make a call take
/// time, panic, or call back into the session it is part of.
pub struct HookedLedger {
    pub inner: MemoryLedger,
    pub on_read: Mutex<Box<dyn FnMut() + Send>>,
    /// Panic when the ledger is dropped (unless already unwinding).
    pub panic_on_drop: bool,
}

impl HookedLedger {
    pub fn new(on_read: impl FnMut() + Send + 'static) -> Self {
        Self {
            inner: MemoryLedger::new(),
            on_read: Mutex::new(Box::new(on_read)),
            panic_on_drop: false,
        }
    }
}

impl Drop for HookedLedger {
    fn drop(&mut self) {
        assert!(
            !self.panic_on_drop || std::thread::panicking(),
            "the ledger panics in its destructor"
        );
    }
}

impl Ledger for HookedLedger {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        // A hook that panics poisons the lock; a later read goes on to the hook again.
        let mut hook = self.on_read.lock().unwrap_or_else(PoisonError::into_inner);
        (hook)();
        drop(hook);
        self.inner.read()
    }

    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError> {
        self.inner.write(expected, change)
    }

    fn write_fenced(
        &self,
        fence: Fence,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError> {
        self.inner.write_fenced(fence, expected, change)
    }

    fn acquire(&self, holder: &str, ttl: Duration) -> Result<Lease, LeaseError> {
        self.inner.acquire(holder, ttl)
    }

    fn acquire_if(
        &self,
        holder: &str,
        ttl: Duration,
        token: &Token,
        fence: Fence,
    ) -> Result<Lease, LeaseError> {
        self.inner.acquire_if(holder, ttl, token, fence)
    }

    fn renew(&self, fence: Fence, ttl: Duration) -> Result<Lease, LedgerError> {
        self.inner.renew(fence, ttl)
    }

    fn release(&self, fence: Fence) -> Result<(), LedgerError> {
        self.inner.release(fence)
    }
}
