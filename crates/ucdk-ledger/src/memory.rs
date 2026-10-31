// SPDX-License-Identifier: Apache-2.0
//! The in-memory ledger.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::doc::Doc;
use crate::lease::{Clock, Fence, Lease, LeaseError, SystemClock};
use crate::record::{Change, Ledger, LedgerError, Snapshot, Token};

#[derive(Debug)]
struct Inner {
    document: Mutex<Doc>,
    clock: Arc<dyn Clock>,
}

/// A ledger in memory. A clone shares the document (and the clock) with the original, so two
/// writers (or a writer and a test) can hold the same ledger.
#[derive(Debug, Clone)]
pub struct MemoryLedger {
    inner: Arc<Inner>,
}

impl Default for MemoryLedger {
    fn default() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }
}

impl MemoryLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// A ledger whose leases are timed by `clock`.
    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Arc::new(Inner {
                document: Mutex::new(Doc::default()),
                clock,
            }),
        }
    }

    fn document(&self) -> MutexGuard<'_, Doc> {
        self.inner
            .document
            .lock()
            .expect("the lock is never poisoned")
    }

    fn put(
        &self,
        fence: Option<Fence>,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError> {
        let mut document = self.document();
        document.admit(self.inner.clock.now_millis(), fence)?;
        if expected.0 != document.version {
            return Err(LedgerError::Stale);
        }
        Ok(Token(document.apply(change)))
    }
}

impl Ledger for MemoryLedger {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        let document = self.document();
        Ok(document.snapshot(self.inner.clock.now_millis()))
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
        self.document()
            .acquire(self.inner.clock.now_millis(), holder, ttl)
    }

    fn acquire_if(
        &self,
        holder: &str,
        ttl: Duration,
        token: &Token,
        fence: Fence,
    ) -> Result<Lease, LeaseError> {
        self.document()
            .acquire_if(self.inner.clock.now_millis(), holder, ttl, token, fence)
    }

    fn renew(&self, fence: Fence, ttl: Duration) -> Result<Lease, LedgerError> {
        self.document()
            .renew(self.inner.clock.now_millis(), fence, ttl)
    }

    fn release(&self, fence: Fence) -> Result<(), LedgerError> {
        self.document().release(fence)
    }
}
