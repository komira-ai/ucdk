// SPDX-License-Identifier: Apache-2.0
//! The executable specification of the release seams: working in-memory implementations with
//! fault injection and call logs. They are deterministic and hold no clock of their own, no I/O
//! and no `HashMap`; time moves only when a test moves the `ManualClock` they share with the
//! ledger under test.
//!
//! - [`FakeBuildSystem`]: the layering is the real one, and the bytes it builds are a function
//!   of the layered command, so a changed flag, environment value or stamp changes them.
//! - [`FakeProcessRunner`]: scripted by argument vector and environment; an unscripted call
//!   fails the test.
//! - [`FakeOutputFiles`]: an in-memory tree of files, directories and symlinks.
//! - [`FakeStore`]: the keyed tree a set is written into.
//! - [`FakeRegistry`] and [`FakeGrantSource`]: a registry that checks every grant it is given,
//!   and the issuer of those grants.
//! - [`FakeSource`]: version control answers, `Unknown` included.
//! - [`FakePause`] and [`FakeHeartbeat`]: counted polls and counted beats.

mod build_system;
mod grant_source;
mod output_files;
mod process_runner;
mod registry;
mod source;
mod store;
mod time;
mod token;

pub use build_system::FakeBuildSystem;
pub use grant_source::{Exchange, FakeGrantSource, JobClaims};
pub use output_files::FakeOutputFiles;
pub use process_runner::{FakeProcessRunner, RunCall};
pub use registry::{FakeRegistry, Read, Scopes};
pub use source::{FakeSource, SourceFault};
pub use store::FakeStore;
pub use time::{FakeHeartbeat, FakePause};
pub use token::coordinate_key;
