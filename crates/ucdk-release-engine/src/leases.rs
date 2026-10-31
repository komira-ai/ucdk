// SPDX-License-Identifier: Apache-2.0
//! The run's lease, held for the length of one request and kept alive on the progress the build
//! system reports. The lease is a [`LeaseGuard`] behind a `RefCell`: the engine uses it between
//! calls to the build system, and the build system uses it, through [`Heartbeat`], during one.
//! The seam has no timer and the engine no thread, so with no progress there is no renewal; a
//! result that comes after a silence longer than the lease is recorded by taking the lease again
//! when nothing else happened to the ledger ([`Leases::settle`]).

use std::cell::RefCell;

use ucdk_ledger::{Change, GuardError, LeaseGuard};
use ucdk_release_seams::{Heartbeat, Stop};

pub(crate) struct Leases<'a> {
    guard: RefCell<LeaseGuard<'a>>,
}

impl<'a> Leases<'a> {
    pub(crate) fn new(guard: LeaseGuard<'a>) -> Self {
        Self {
            guard: RefCell::new(guard),
        }
    }

    /// Writes `change` under the lease.
    pub(crate) fn commit(&self, change: &Change) -> Result<(), GuardError> {
        self.guard.borrow_mut().commit(change)
    }

    /// Extends the lease. Done before each effect: here, before each build-system invocation.
    pub(crate) fn renew(&self) -> Result<(), GuardError> {
        self.guard.borrow_mut().renew()
    }

    /// Makes the lease good for a write that records the result of work already done. It is
    /// renewed; when it has lapsed, and no one wrote the ledger or was granted the lease since
    /// the guard's own last write, it is taken again under a new fence. Anything else is a lost
    /// lease. Never used before an intent or an effect: nothing starts under a lease taken here.
    pub(crate) fn settle(&self) -> Result<(), GuardError> {
        let mut guard = self.guard.borrow_mut();
        match guard.renew() {
            Err(GuardError::Lost) => {
                let token = guard.token().clone();
                let fence = guard.fence();
                guard.reacquire_if_unchanged(&token, fence)
            }
            other => other,
        }
    }

    /// Gives the lease back. A release that cannot be confirmed leaves the lease to lapse, and
    /// the same holder takes it again at once, so nothing waits on it.
    pub(crate) fn release(&self) {
        let _ = self.guard.borrow_mut().release();
    }
}

impl Heartbeat for Leases<'_> {
    /// A progress event of the invocation in flight: renews when less than half the lease is
    /// left. `Stop` when the lease cannot be kept, so the runner kills what it started.
    fn beat(&self) -> Result<(), Stop> {
        self.guard.borrow_mut().renew_if_due().map_err(|_| Stop)
    }
}
