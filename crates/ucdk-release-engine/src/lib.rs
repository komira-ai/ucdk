// SPDX-License-Identifier: Apache-2.0
//! The release engine. A request names a stage and what to run in it (`--only action:<name>` or
//! `--only validation:<name>`, or all of it), and the engine runs the Test actions and the
//! validations that selects, through the seams of `ucdk-release-seams`, and writes what it did
//! to the run's ledger through the codec of `ucdk-release`.
//!
//! * A **Test action** runs the machine's test targets through the build system, one row per
//!   target; a target that ran nothing is not a pass ([`test_action`] holds the rules).
//! * A **validation** is evaluated by the engine itself: `SET_CLOSURE` over a sealed set read
//!   from the store, `CHANNEL_READBACK` over a channel read through its publisher
//!   ([`validation`] holds the rules).
//!
//! The run's lease is taken after every refusal that needs no ledger, renewed before each
//! invocation, kept alive on each progress event the build system reports, and given back
//! however the request ended. A result that comes after a silence longer than the lease is
//! recorded by taking the lease again when no one else touched the ledger.
//!
//! The engine reads no clock: time reaches it through the ledger, and a poll is a counted
//! [`Pause`](ucdk_release_seams::Pause). The only effect of its own is the directory it makes
//! under the scratch directory for the report of each test invocation, and removes after it.

mod engine;
mod leases;
mod outcome;
mod records;
mod set;
mod test_action;
mod validation;

pub use engine::{Engine, Request, Run, Seams};
pub use outcome::Outcome;
pub use set::SET_FILE;
pub use test_action::TEST_ASSERTION;
pub use validation::READBACK_ASSERTION;
