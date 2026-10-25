// SPDX-License-Identifier: Apache-2.0
//! The ledger: what the engine records about every object it owns.
//!
//! A [`Record`] says which node an object is, who owns it (an [`OwnerPath`], not a flat id),
//! how it came to be ([`Origin`]), where it stands ([`State`]) and what it needed. The
//! [`Ledger`] trait is a single document with a version [`Token`]: a write names the token it
//! read and fails with [`LedgerError::Stale`] when another writer got there first, so two
//! engines can never overwrite each other unseen. [`MemoryLedger`] keeps the document in
//! memory; [`FileLedger`] keeps it in a file on one machine, replaced atomically on every write,
//! so a run can be resumed after the process died.
//!
//! The ledger holds no secrets and no cloud state beyond the physical id and the node's
//! identity. It is not pure (it is state, and a real one does I/O), so it sits above the pure
//! crates and below the engine.
//!
//! A ledger also keeps a lease ([`lease`]): a holder takes it with an identity and an expiry and
//! gets a [`Fence`] that only goes up, and a write under a fence that is not the current one is
//! refused with [`LedgerError::Fenced`], so a stalled holder cannot write after another took over.

mod doc;
mod file;
mod format;
pub mod lease;
mod memory;
mod record;

pub use file::{FileLedger, OpenError};
pub use lease::{Clock, Fence, Lease, LeaseError, MAX_CLOCK_SKEW, ManualClock, SystemClock};
pub use memory::MemoryLedger;
pub use record::{Change, Ledger, LedgerError, Origin, OwnerPath, Record, Snapshot, State, Token};
