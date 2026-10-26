// SPDX-License-Identifier: Apache-2.0
//! The ledger: what a writer records about every object it owns.
//!
//! A [`Record`] says what a thing is (a path, a tag and a readable name), who owns it (an
//! [`OwnerPath`], not a flat id), how it came to be ([`Origin`]), where it stands ([`State`])
//! and what it needed. The [`Ledger`] trait is a single document with a version [`Token`]: a
//! write names the token it read and fails with [`LedgerError::Stale`] when another writer got
//! there first, so two writers can never overwrite each other unseen. [`MemoryLedger`] keeps the
//! document in memory; [`FileLedger`] keeps it in a file on one machine, replaced atomically on
//! every write, so a run can be resumed after the process died. A file can also be opened
//! read-only ([`FileLedger::open_read_only`]), which creates, locks and removes nothing.
//!
//! The ledger holds no secrets. It is not pure (it is state, and a real one does I/O), so it
//! sits above the pure crates.
//!
//! A ledger also keeps a lease ([`lease`]): a holder takes it with an identity and an expiry and
//! gets a [`Fence`] that only goes up, and a write under a fence that is not the current one is
//! refused with [`LedgerError::Fenced`], so a stalled holder cannot write after another took
//! over. [`LeaseGuard`] ([`guard`]) is the lifecycle of one holder's lease over these calls.

mod doc;
mod file;
mod format;
pub mod guard;
pub mod lease;
mod memory;
mod record;

pub use file::{FileLedger, OpenError};
pub use format::escape_token;
pub use guard::{DEFAULT_LEASE_TTL, GuardError, LeaseGuard, default_holder};
pub use lease::{
    Clock, Fence, Holding, Lease, LeaseError, LeaseState, MAX_CLOCK_SKEW, ManualClock, SystemClock,
};
pub use memory::MemoryLedger;
pub use record::{Change, Ledger, LedgerError, Origin, OwnerPath, Record, Snapshot, State, Token};
