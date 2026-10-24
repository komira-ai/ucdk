// SPDX-License-Identifier: Apache-2.0
//! The ledger: what the engine records about every object it owns.
//!
//! A [`Record`] says which node an object is, who owns it (an [`OwnerPath`], not a flat id),
//! how it came to be ([`Origin`]), where it stands ([`State`]) and what it needed. The
//! [`Ledger`] trait is a single document with a version [`Token`]: a write names the token it
//! read and fails with [`LedgerError::Stale`] when another writer got there first, so two
//! engines can never overwrite each other unseen. [`MemoryLedger`] keeps the document in
//! memory.
//!
//! The ledger holds no secrets and no cloud state beyond the physical id and the node's
//! identity. It is not pure (it is state, and a real one does I/O), so it sits above the pure
//! crates and below the engine.
//!
//! [`lease::Lease`] is an interface only: it names what a holder of the cell will need once
//! lease time and fencing exist, and nothing implements it yet.

pub mod lease;
mod memory;
mod record;

pub use memory::MemoryLedger;
pub use record::{Change, Ledger, LedgerError, Origin, OwnerPath, Record, Snapshot, State, Token};
