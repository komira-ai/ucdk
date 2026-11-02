// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]
//! Helpers shared by the ledger's tests: a scratch directory that cleans up after itself, the
//! records the tests write, both ledgers behind one type, a ledger that can be made to fail,
//! and a counter with a hook that stands in for the calls a lease guards.

pub mod release;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ucdk_core::{Clock, Locator, ManualClock, RecordPath, Tag};
use ucdk_ledger::{
    Change, Fence, FileLedger, Lease, LeaseError, Ledger, LedgerError, MemoryLedger, Record,
    Snapshot, Token,
};

/// A directory under the build's scratch area, empty on creation and removed on drop.
#[derive(Debug)]
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("ledger-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("make the scratch directory");
        Self(dir)
    }

    pub fn dir(&self) -> &Path {
        &self.0
    }

    /// The ledger file of the directory.
    pub fn file(&self) -> PathBuf {
        self.0.join("ledger.ucdk")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn path(text: &str) -> RecordPath {
    RecordPath::new(text).expect("a valid path")
}

pub fn record(text: &str) -> Record {
    Record::intent(path(text), Tag::new("service"), "name", BTreeSet::new())
}

pub fn confirmed(text: &str, id: &str) -> Record {
    record(text).confirmed(Locator::new(id))
}

/// Writes `change` against the current token.
pub fn put(ledger: &dyn Ledger, change: Change) {
    let token = ledger.read().expect("read").token;
    ledger.write(&token, change).expect("write");
}

/// FNV-1a, the checksum of the file format, so a test can seal a document it wrote by hand.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A document made of `lines` and a correct `end` line claiming `records` records.
pub fn sealed(lines: &[&str], records: usize) -> Vec<u8> {
    let mut body = lines.join("\n");
    body.push('\n');
    let end = format!("end {records} {:016x}\n", fnv(body.as_bytes()));
    body.push_str(&end);
    body.into_bytes()
}

// ---- the ledgers the scenarios run on ---------------------------------------------------------

/// The ledger implementations the shared scenarios run against.
#[derive(Debug, Clone, Copy)]
pub enum Backend {
    Memory,
    File,
}

impl Backend {
    /// A new, empty ledger of this kind, on a clock that never moves: a lease on it never
    /// lapses, and no test reads the time.
    pub fn ledger(self) -> AnyLedger {
        self.ledger_on(Arc::new(ManualClock::at(1_000_000)))
    }

    /// A new, empty ledger of this kind, whose leases are timed by `clock`.
    pub fn ledger_on(self, clock: Arc<dyn Clock>) -> AnyLedger {
        match self {
            Self::Memory => AnyLedger::Memory(MemoryLedger::with_clock(clock)),
            Self::File => {
                static NEXT: AtomicU64 = AtomicU64::new(0);
                let n = NEXT.fetch_add(1, Ordering::SeqCst);
                let dir = Scratch::new(&format!("any-{n}"));
                let ledger =
                    FileLedger::open_with_clock(dir.file(), clock).expect("open the file ledger");
                AnyLedger::File(ledger, Arc::new(dir))
            }
        }
    }
}

/// A ledger of either kind. A clone shares the document, as a clone of each kind does, and the
/// scratch directory of a file ledger lives until the last clone is dropped.
#[derive(Debug, Clone)]
pub enum AnyLedger {
    Memory(MemoryLedger),
    File(FileLedger, Arc<Scratch>),
}

impl AnyLedger {
    fn inner(&self) -> &dyn Ledger {
        match self {
            Self::Memory(ledger) => ledger,
            Self::File(ledger, _) => ledger,
        }
    }
}

impl Ledger for AnyLedger {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        self.inner().read()
    }

    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError> {
        self.inner().write(expected, change)
    }

    fn write_fenced(
        &self,
        fence: Fence,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError> {
        self.inner().write_fenced(fence, expected, change)
    }

    fn acquire(&self, holder: &str, ttl: Duration) -> Result<Lease, LeaseError> {
        self.inner().acquire(holder, ttl)
    }

    fn renew(&self, fence: Fence, ttl: Duration) -> Result<Lease, LedgerError> {
        self.inner().renew(fence, ttl)
    }

    fn release(&self, fence: Fence) -> Result<(), LedgerError> {
        self.inner().release(fence)
    }
}

// ---- a ledger that can be made to fail -------------------------------------------------------

/// A ledger that counts write attempts and can refuse the first `n` of them as stale, fail every
/// read, fail every acquire, take the lease and lose the answer, or fail every release.
#[derive(Debug)]
pub struct FlakyLedger {
    inner: AnyLedger,
    writes: AtomicU64,
    stale_writes: AtomicU64,
    fail_reads: AtomicBool,
    fail_acquire: AtomicBool,
    lose_acquire_answer: AtomicBool,
    fail_release: AtomicBool,
}

impl FlakyLedger {
    pub fn on(backend: Backend) -> Self {
        Self::over(backend.ledger())
    }

    /// A flaky ledger of `backend` whose leases are timed by `clock`.
    pub fn on_clock(backend: Backend, clock: Arc<dyn Clock>) -> Self {
        Self::over(backend.ledger_on(clock))
    }

    fn over(inner: AnyLedger) -> Self {
        Self {
            inner,
            writes: AtomicU64::default(),
            stale_writes: AtomicU64::default(),
            fail_reads: AtomicBool::default(),
            fail_acquire: AtomicBool::default(),
            lose_acquire_answer: AtomicBool::default(),
            fail_release: AtomicBool::default(),
        }
    }

    /// Counts a write attempt, and refuses it as stale while the test asks for that.
    fn gate(&self) -> Result<(), LedgerError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        let left = self.stale_writes.load(Ordering::SeqCst);
        if left > 0 {
            self.stale_writes.store(left - 1, Ordering::SeqCst);
            return Err(LedgerError::Stale);
        }
        Ok(())
    }

    /// Makes the next `n` write attempts answer stale, without writing.
    pub fn stale_for(&self, n: u64) {
        self.stale_writes.store(n, Ordering::SeqCst);
    }

    /// The write attempts made so far, refused ones included.
    pub fn writes(&self) -> u64 {
        self.writes.load(Ordering::SeqCst)
    }

    pub fn fail_reads(&self, on: bool) {
        self.fail_reads.store(on, Ordering::SeqCst);
    }

    /// Makes every acquire fail as unavailable.
    pub fn fail_acquire(&self, on: bool) {
        self.fail_acquire.store(on, Ordering::SeqCst);
    }

    /// Makes every acquire take the lease and then fail as unavailable, as when the grant
    /// landed and its answer was lost.
    pub fn lose_acquire_answer(&self, on: bool) {
        self.lose_acquire_answer.store(on, Ordering::SeqCst);
    }

    /// Makes every release fail as unavailable, whether or not the lease is given back.
    pub fn fail_release(&self, on: bool) {
        self.fail_release.store(on, Ordering::SeqCst);
    }
}

impl Ledger for FlakyLedger {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(LedgerError::Unavailable("reads are down".to_owned()));
        }
        self.inner.read()
    }

    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError> {
        self.gate()?;
        self.inner.write(expected, change)
    }

    fn write_fenced(
        &self,
        fence: Fence,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError> {
        self.gate()?;
        self.inner.write_fenced(fence, expected, change)
    }

    fn acquire(&self, holder: &str, ttl: Duration) -> Result<Lease, LeaseError> {
        if self.fail_acquire.load(Ordering::SeqCst) {
            return Err(LeaseError::Unavailable("the lease is down".to_owned()));
        }
        let lease = self.inner.acquire(holder, ttl)?;
        if self.lose_acquire_answer.load(Ordering::SeqCst) {
            return Err(LeaseError::Unavailable("the answer was lost".to_owned()));
        }
        Ok(lease)
    }

    fn renew(&self, fence: Fence, ttl: Duration) -> Result<Lease, LedgerError> {
        self.inner.renew(fence, ttl)
    }

    fn release(&self, fence: Fence) -> Result<(), LedgerError> {
        if self.fail_release.load(Ordering::SeqCst) {
            return Err(LedgerError::Unavailable("the release was lost".to_owned()));
        }
        self.inner.release(fence)
    }
}

// ---- the calls a lease guards ----------------------------------------------------------------

/// Stands in for a call that has an effect outside the ledger (the work a lease protects): it
/// counts the calls made, so a test can say how many a holder got to make.
#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    /// Makes one call; the number of this call, from 1.
    pub fn call(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// The calls made so far.
    pub fn calls(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A [`Counter`] that runs a hook once, in the middle of its `at`th call: the moment a test
/// does something to the ledger (a second holder arrives, a clock moves) while the first
/// holder's work is under way.
pub struct Hooked {
    counter: Counter,
    at: u64,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Hooked {
    pub fn new(at: u64, hook: impl FnOnce() + Send + 'static) -> Self {
        Self {
            counter: Counter::default(),
            at,
            hook: Mutex::new(Some(Box::new(hook))),
        }
    }

    /// Makes one call, running the hook when it is the `at`th.
    pub fn call(&self) -> u64 {
        let n = self.counter.call();
        if n == self.at {
            let hook = self.hook.lock().expect("lock").take();
            if let Some(hook) = hook {
                hook();
            }
        }
        n
    }

    pub fn calls(&self) -> u64 {
        self.counter.calls()
    }
}
