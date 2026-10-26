// SPDX-License-Identifier: Apache-2.0
//! The values every other crate of ucdk shares: record paths and keys, the tags and locators a
//! ledger record carries, a 256-bit hash with a length-framed way to feed it, dependency order,
//! and the clock a ledger reads.
//!
//! This crate is pure and synchronous: it holds no I/O, no wall clock and no `HashMap`, so the
//! same inputs give the same results in the same order. The one reader of the wall clock is
//! `SystemClock`, in `ucdk-ledger`.

mod clock;
mod graph;
mod hash;
mod ids;

pub use clock::{Clock, ManualClock, millis};
pub use graph::order;
pub use hash::{Framed, Hash256};
pub use ids::{
    Locator, ModelError, NODE_KEY_LEN, OwnerLabel, RecordKey, RecordPath, SEGMENT_MAX, Tag,
};
