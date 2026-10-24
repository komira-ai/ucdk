// SPDX-License-Identifier: Apache-2.0
//! From a resource list to a plan, with no side effect.
//!
//! This crate is pure: it holds no I/O, no clock and no `HashMap`, and it calls no provider.
//! Everything it reads is plain data handed to it, so the same inputs give the same results
//! in the same order.
//!
//! - [`catalog`] checks a `ucdk.resource.v1` list ([`catalog::validate`] collects every
//!   finding, refusing references to resources that are not there and cycles) and lowers it
//!   ([`catalog::lower`]) to the nodes a provider works on, in dependency order.
//! - [`graph`] is the ordering both use (Kahn's algorithm, ties broken by name).
//! - [`plan`] compares the lowered nodes with an [`Observed`] state (what the cloud shows and
//!   what the ledger records, as data) and returns a [`Plan`]: the creates, updates and
//!   deletes to make, in order, with a [`PlanHash`] an approver can pin. A foreign or
//!   conflicting object refuses the whole run before any step is produced.
//!
//! Applying a plan (the ledger, the engine, rollback) is not here.

pub mod catalog;
pub mod graph;
mod hash;
pub mod observed;
mod plan;

pub use catalog::{Finding, FindingKind, Lowered, PlanNode, ValueRef, lower, validate};
pub use hash::PlanHash;
pub use observed::{LedgerToken, LiveState, Observed, Record, Seen};
pub use plan::{Action, Conflict, ConflictKind, Plan, Refused, Retained, Step, plan};
