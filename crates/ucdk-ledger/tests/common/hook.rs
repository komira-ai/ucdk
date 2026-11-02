// SPDX-License-Identifier: Apache-2.0
//! A ledger that runs a closure between a grant and the first read after it: the moment a
//! stalled caller lets the world move on, made deterministic.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ucdk_ledger::{Change, Fence, Lease, LeaseError, Ledger, LedgerError, Snapshot, Token};

type Hook<'a> = Box<dyn FnOnce() + Send + 'a>;

/// Wraps `inner`. The first `read` after a successful `acquire` runs the closure, then reads.
pub struct AfterGrant<'a> {
    inner: &'a dyn Ledger,
    granted: AtomicBool,
    hook: Mutex<Option<Hook<'a>>>,
}

impl<'a> AfterGrant<'a> {
    pub fn new(inner: &'a dyn Ledger, hook: impl FnOnce() + Send + 'a) -> Self {
        Self {
            inner,
            granted: AtomicBool::default(),
            hook: Mutex::new(Some(Box::new(hook))),
        }
    }
}

impl Ledger for AfterGrant<'_> {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        if self.granted.load(Ordering::SeqCst) {
            let hook = self.hook.lock().expect("lock").take();
            if let Some(hook) = hook {
                hook();
            }
        }
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
        let lease = self.inner.acquire(holder, ttl)?;
        self.granted.store(true, Ordering::SeqCst);
        Ok(lease)
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
