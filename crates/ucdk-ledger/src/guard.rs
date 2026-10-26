// SPDX-License-Identifier: Apache-2.0
//! The lifecycle of one holder's lease: take it, write under its fence, renew it, give it back.
//!
//! A [`LeaseGuard`] is what a long-running writer holds between taking the lease of a ledger and
//! giving it back. It takes no clock: the ledger owns the clock and judges expiry, so the guard
//! only reports what the ledger answered. What it adds to the bare [`Ledger`] calls is the
//! classification of every answer by what the caller may assume:
//!
//! * [`GuardError::Held`]: another holder has the lease; nothing changed.
//! * [`GuardError::Lost`]: the lease was taken over, released or lapsed. Every later call
//!   answers `Lost` and writes nothing, so a holder that lost its lease cannot write over the
//!   new holder's work.
//! * [`GuardError::Unconfirmed`]: a grant or a release whose answer was lost. The lease may
//!   still be held; until it lapses a different holder is refused, and this holder takes it
//!   again at once. Treat it as an effect that may have landed.
//! * [`GuardError::Busy`]: another writer kept changing the document through every retry.

use std::fmt;
use std::time::Duration;

use crate::lease::{Fence, LeaseError};
use crate::record::{Change, Ledger, LedgerError};

/// How long a lease lasts when the caller has no reason to choose: the holder renews before each
/// step of its work, so one step has this long.
pub const DEFAULT_LEASE_TTL: Duration = Duration::from_secs(600);

/// How many times a write that finds the document changed is read and tried again.
const WRITE_ATTEMPTS: usize = 5;

/// The name a holder takes the lease as when none is given: the process id and a random
/// number, so two holders are told apart even when their process ids are equal (two containers
/// on one ledger are both process 1).
pub fn default_holder() -> String {
    use std::hash::{BuildHasher, Hasher as _};
    let random = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    format!("ucdk-{}-{random:016x}", std::process::id())
}

/// Why a call on a [`LeaseGuard`] did not do what it was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardError {
    /// Another holder has the lease, or its lapse is not yet past the skew bound. Nothing
    /// changed. `remaining` is the longest the caller can need to wait.
    Held { holder: String, remaining: Duration },
    /// The lease was taken over, released or lapsed, so the ledger refused the write or the
    /// renewal. Every later call answers `Lost` and writes nothing.
    Lost,
    /// A grant or a release failed with an answer that does not say whether the ledger changed:
    /// the lease may still be held.
    Unconfirmed { why: String },
    /// Any other failure of the ledger.
    Ledger(LedgerError),
    /// The document stayed stale through every retry: another writer keeps changing it.
    Busy,
    /// The guard has given the lease back.
    NotHeld,
}

impl fmt::Display for GuardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Held { holder, remaining } => write!(
                f,
                "the ledger is leased to {holder}; try again when that holder is done \
                 (its lease can be taken within {} second(s) at most)",
                remaining.as_secs().max(1)
            ),
            Self::Lost => f.write_str(
                "the lease was lost (it lapsed, or another holder took it): nothing more is written",
            ),
            Self::Unconfirmed { why } => write!(
                f,
                "the lease may still be held ({why}): act again as the same holder, \
                 or wait for the lease to lapse"
            ),
            Self::Ledger(e) => write!(f, "ledger: {e}"),
            Self::Busy => f.write_str("the ledger kept changing under the writer"),
            Self::NotHeld => f.write_str("the lease is not held"),
        }
    }
}

impl std::error::Error for GuardError {}

/// Where the guard stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    Held,
    Lost,
    Released,
}

/// A held lease. It borrows the ledger it was taken on and is given back with
/// [`LeaseGuard::release`], however the work it guards ended; dropping it gives nothing back,
/// and the lease then lapses on its own.
pub struct LeaseGuard<'a> {
    ledger: &'a dyn Ledger,
    holder: String,
    ttl: Duration,
    fence: Fence,
    standing: Standing,
}

impl<'a> LeaseGuard<'a> {
    /// Takes the lease of `ledger` as `holder` for `ttl`. Do this after every refusal that
    /// needs no ledger, and before the first write.
    ///
    /// # Errors
    ///
    /// [`GuardError::Held`] when another holder has it, [`GuardError::Unconfirmed`] when the
    /// ledger could not say whether the grant landed, [`GuardError::Ledger`] otherwise.
    pub fn acquire(
        ledger: &'a dyn Ledger,
        holder: impl Into<String>,
        ttl: Duration,
    ) -> Result<Self, GuardError> {
        let holder = holder.into();
        match ledger.acquire(&holder, ttl) {
            Ok(lease) => Ok(Self {
                ledger,
                holder,
                ttl,
                fence: lease.fence,
                standing: Standing::Held,
            }),
            Err(LeaseError::Held { holder, remaining }) => {
                Err(GuardError::Held { holder, remaining })
            }
            // The grant may have landed with its answer lost: the lease may be ours.
            Err(LeaseError::Unavailable(why)) => Err(GuardError::Unconfirmed { why }),
            Err(LeaseError::ReadOnly) => Err(GuardError::Ledger(LedgerError::ReadOnly)),
        }
    }

    /// Who the lease is taken as.
    pub fn holder(&self) -> &str {
        &self.holder
    }

    /// The fence of the lease, which every write of the guard is made under.
    pub fn fence(&self) -> Fence {
        self.fence
    }

    fn check(&self) -> Result<(), GuardError> {
        match self.standing {
            Standing::Held => Ok(()),
            Standing::Lost => Err(GuardError::Lost),
            Standing::Released => Err(GuardError::NotHeld),
        }
    }

    /// Maps a ledger error to the guard's, and remembers a lost lease.
    fn classify(&mut self, error: LedgerError) -> GuardError {
        match error {
            LedgerError::Fenced | LedgerError::LeaseLapsed => {
                self.standing = Standing::Lost;
                GuardError::Lost
            }
            other => GuardError::Ledger(other),
        }
    }

    /// Writes `change` under the lease's fence, reading the document again and trying again
    /// when another writer got in between: a change of one record does not depend on the
    /// others. Up to five attempts are made on a stale token, then the answer is
    /// [`GuardError::Busy`].
    ///
    /// # Errors
    ///
    /// [`GuardError::Lost`] when the ledger refuses the fence (and for every call after),
    /// [`GuardError::NotHeld`] after the lease was given back, [`GuardError::Busy`], or
    /// [`GuardError::Ledger`] for any other failure.
    pub fn commit(&mut self, change: &Change) -> Result<(), GuardError> {
        self.check()?;
        for _ in 0..WRITE_ATTEMPTS {
            let token = self.ledger.read().map_err(|e| self.classify(e))?.token;
            match self.ledger.write_fenced(self.fence, &token, change.clone()) {
                Ok(_) => return Ok(()),
                Err(LedgerError::Stale) => {}
                Err(other) => return Err(self.classify(other)),
            }
        }
        Err(GuardError::Busy)
    }

    /// Extends the lease to the guard's `ttl` from now. A holder does this before each piece of
    /// work that must not outlast it.
    ///
    /// # Errors
    ///
    /// As [`LeaseGuard::commit`], without `Busy`.
    pub fn renew(&mut self) -> Result<(), GuardError> {
        self.check()?;
        match self.ledger.renew(self.fence, self.ttl) {
            Ok(_) => Ok(()),
            Err(error) => Err(self.classify(error)),
        }
    }

    /// Gives the lease back, so another holder may take it at once. A guard whose lease was
    /// lost gives nothing back and answers `Lost`.
    ///
    /// # Errors
    ///
    /// [`GuardError::Unconfirmed`] when the ledger could not say whether the release landed
    /// (the guard stays held, so the call can be made again), [`GuardError::Lost`] when the
    /// lease was no longer this guard's, [`GuardError::NotHeld`] when it was given back already.
    pub fn release(&mut self) -> Result<(), GuardError> {
        self.check()?;
        match self.ledger.release(self.fence) {
            Ok(()) => {
                self.standing = Standing::Released;
                Ok(())
            }
            Err(LedgerError::Unavailable(why)) => Err(GuardError::Unconfirmed { why }),
            Err(error) => Err(self.classify(error)),
        }
    }
}
