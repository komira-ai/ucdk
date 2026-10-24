// SPDX-License-Identifier: Apache-2.0
//! The lease, as an interface only. Time, expiry and fencing are not here: no type in this
//! change implements [`Lease`], and the engine does not call it.

/// The generation of a lease. A later holder has a higher epoch, which is what fencing
/// compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Epoch(pub u64);

/// Why a lease was not granted or not released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseError {
    /// Another holder has the cell.
    Held { holder: String },
    /// The lease could not be reached.
    Unavailable(String),
}

/// Exclusive use of a cell's ledger for one holder at a time.
pub trait Lease: Send + Sync {
    /// Takes the lease for `holder`.
    fn acquire(&self, holder: &str) -> Result<Epoch, LeaseError>;

    /// Gives the lease back.
    fn release(&self, epoch: Epoch) -> Result<(), LeaseError>;
}
