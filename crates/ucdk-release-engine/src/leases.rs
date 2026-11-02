// SPDX-License-Identifier: Apache-2.0
//! The run's lease, held for the length of one request and kept alive on the progress the build
//! system reports. The lease is a [`LeaseGuard`] behind a `RefCell`: the engine uses it between
//! calls to the build system, and the build system uses it, through [`Heartbeat`], during one.
//! The seam has no timer and the engine no thread, so with no progress there is no renewal; a
//! result that comes after a silence longer than the lease is recorded by taking the lease again
//! when nothing else happened to the ledger ([`Leases::settle`]).
//!
//! A progress event that cannot keep the lease asks the runner to stop, and the reason is kept
//! with its type ([`Leases::beat_failure`]): a lease another holder took is a lost lease, and a
//! ledger that could not be read or written is a ledger failure, not a lost lease.

use std::cell::RefCell;

use ucdk_ledger::{Change, GuardError, LeaseGuard};
use ucdk_release_seams::{Heartbeat, Stop};

pub(crate) struct Leases<'a> {
    guard: RefCell<LeaseGuard<'a>>,
    /// Why the last progress event asked the runner to stop, until the engine takes it.
    beat: RefCell<Option<GuardError>>,
}

impl<'a> Leases<'a> {
    pub(crate) fn new(guard: LeaseGuard<'a>) -> Self {
        Self {
            guard: RefCell::new(guard),
            beat: RefCell::new(None),
        }
    }

    /// Writes `change` under the lease.
    pub(crate) fn commit(&self, change: &Change) -> Result<(), GuardError> {
        self.guard.borrow_mut().commit(change)
    }

    /// Makes the lease good again after work that ran without a progress event: it is
    /// renewed; when it has lapsed, and no one wrote the ledger or was granted the lease since
    /// the guard's own last write, it is taken again under a new fence. Anything else is a lost
    /// lease. The new grant is conditional on nothing having happened since the guard's own
    /// last write, so what was done under the old fence and what is done under the new one are
    /// never interleaved with another holder's work. It is used after every result, passed or
    /// not, so a lease taken over is reported as lost whatever the work came to, and between
    /// the targets of a Test action, so a silent target does not make the next one fail.
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

    /// Why a progress event of the invocation that just returned asked the runner to stop, if
    /// one did. Taking it clears it.
    pub(crate) fn beat_failure(&self) -> Option<GuardError> {
        self.beat.borrow_mut().take()
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
        self.guard.borrow_mut().renew_if_due().map_err(|e| {
            self.beat.borrow_mut().get_or_insert(e);
            Stop
        })
    }
}
