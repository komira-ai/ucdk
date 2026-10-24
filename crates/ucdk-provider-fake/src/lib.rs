// SPDX-License-Identifier: Apache-2.0
//! An in-memory provider with fault injection. It implements the real [`Provider`] trait and
//! is the executable spec of it: a behaviour change is made here and in the contract kit
//! first, and a real provider is brought to match.
//!
//! What it models: a store of objects ordered by physical id, labels exactly as written, a
//! store of fields the catalog does not model, and an ordered log of every call. Stamps take
//! the two shapes of the trait: a native tag written in the create call, and an encoding in
//! the physical name for a kind that cannot hold tags.
//!
//! Faults are constructor data ([`Faults`]): `fail_at_call`, `read_lag`, `foreign`,
//! `race_next`, `ambiguous_success`, `ambiguous_at_call`, `fail_reads`, `create_failed` (objects
//! are born failed) and `converging_reads` (objects report converging for that many reads);
//! all but `foreign` can also be armed on a built fake ([`Fake::faults_mut`]). An object can
//! also be put in a status out of band ([`Fake::set_status`]). A limited variant ([`FakeConfig::limited`]) refuses
//! the kind `job`. Limits and names here are this package's own, never a real cloud's.
//!
//! [`Provider`]: ucdk_provider::Provider

mod config;
mod dump;
mod fake;
mod harness;
mod log;
mod store;

pub use config::{FakeConfig, Faults, FaultsMut, ForeignObject};
pub use fake::Fake;
pub use harness::FakeHarness;
pub use log::{Call, Op};
