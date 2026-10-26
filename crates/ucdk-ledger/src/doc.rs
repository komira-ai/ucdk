// SPDX-License-Identifier: Apache-2.0
//! The document both ledgers keep: the records, their version, and the lease. The rules of
//! the lease are here once, as functions of the time they are given, so the two ledgers cannot
//! drift apart and a test can hold a clock of its own for each side.

use std::collections::BTreeMap;
use std::time::Duration;

use ucdk_provider::NodePath;

use crate::lease::{Fence, Lease, LeaseError, MAX_CLOCK_SKEW, millis};
use crate::record::{Change, LedgerError, Record};

/// The holder of the lease now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Held {
    pub(crate) holder: String,
    pub(crate) expires_at: u64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Doc {
    pub(crate) version: u64,
    /// The fence of the last grant. It never goes down, whether or not a lease is held.
    pub(crate) fence: u64,
    pub(crate) lease: Option<Held>,
    pub(crate) records: BTreeMap<NodePath, Record>,
}

impl Doc {
    fn lease_of(&self, held: &Held) -> Lease {
        Lease {
            holder: held.holder.clone(),
            fence: Fence(self.fence),
            expires_at: held.expires_at,
        }
    }

    /// Grants the lease to `holder` unless another holds it, or lapsed less than the skew
    /// bound ago. The holder named in the lease takes it again at once, live or lapsed, with a
    /// new fence: a run that ended without giving the lease back (a release that failed, a
    /// crash) is not made to wait out its own lease. A grant does not change the version: a
    /// plan made before it stays valid.
    pub(crate) fn acquire(
        &mut self,
        now: u64,
        holder: &str,
        ttl: Duration,
    ) -> Result<Lease, LeaseError> {
        if let Some(held) = self.lease.as_ref().filter(|held| held.holder != holder) {
            let free_at = held.expires_at.saturating_add(millis(MAX_CLOCK_SKEW));
            if now < free_at {
                return Err(LeaseError::Held {
                    holder: held.holder.clone(),
                    remaining: Duration::from_millis(free_at - now),
                });
            }
        }
        self.fence += 1;
        let held = Held {
            holder: holder.to_owned(),
            expires_at: now.saturating_add(millis(ttl)),
        };
        let lease = self.lease_of(&held);
        self.lease = Some(held);
        Ok(lease)
    }

    /// Whether `fence` is the live lease's: the current fence, not yet lapsed at `now`.
    fn check(&self, now: u64, fence: Fence) -> Result<&Held, LedgerError> {
        match &self.lease {
            Some(held) if fence.0 == self.fence => {
                if now < held.expires_at {
                    Ok(held)
                } else {
                    Err(LedgerError::LeaseLapsed)
                }
            }
            _ => Err(LedgerError::Fenced),
        }
    }

    /// Extends the lease of `fence` to `now + ttl`. A lapsed lease is not renewed, even when
    /// nobody has taken it: the holder acquires again and gets a new fence.
    pub(crate) fn renew(
        &mut self,
        now: u64,
        fence: Fence,
        ttl: Duration,
    ) -> Result<Lease, LedgerError> {
        let holder = self.check(now, fence)?.holder.clone();
        let held = Held {
            holder,
            expires_at: now.saturating_add(millis(ttl)),
        };
        let lease = self.lease_of(&held);
        self.lease = Some(held);
        Ok(lease)
    }

    /// Gives the lease of `fence` back, lapsed or not, so another holder may take it at once.
    pub(crate) fn release(&mut self, fence: Fence) -> Result<(), LedgerError> {
        if self.lease.is_some() && fence.0 == self.fence {
            self.lease = None;
            Ok(())
        } else {
            Err(LedgerError::Fenced)
        }
    }

    /// The refusal for a write, if there is one: a fenced write needs the live lease's fence;
    /// an unfenced one is refused while any lease is live (or lapsed inside the skew bound).
    pub(crate) fn admit(&self, now: u64, fence: Option<Fence>) -> Result<(), LedgerError> {
        match (fence, &self.lease) {
            (Some(fence), _) => self.check(now, fence).map(|_| ()),
            (None, Some(held)) if now < held.expires_at.saturating_add(millis(MAX_CLOCK_SKEW)) => {
                Err(LedgerError::Leased {
                    holder: held.holder.clone(),
                })
            }
            (None, _) => Ok(()),
        }
    }

    /// Applies `change` and moves the version.
    pub(crate) fn apply(&mut self, change: Change) -> u64 {
        match change {
            Change::Put(record) => {
                self.records.insert(record.path.clone(), record);
            }
            Change::Remove(path) => {
                self.records.remove(&path);
            }
        }
        self.version += 1;
        self.version
    }
}
