// SPDX-License-Identifier: Apache-2.0
//! The seams of a release: every effect the engine has on the world goes through one of these
//! traits, so each has a fake and a real implementation behind the same contract.
//!
//! - [`SeamError`]: the one classified error. `Refused`, `Permanent`, `RateLimited` and
//!   `GrantRefused` promise that no external effect landed; `Transient`, `InFlight` and
//!   `Unknown` do not, and the caller observes before it goes on.
//! - [`BuildSystem`] over a [`ProcessRunner`] and [`OutputFiles`]: builds and tests.
//! - [`ArtifactStore`] and [`EntryWriter`]: the keyed tree a set is written into.
//! - [`Publisher`] and [`GrantSource`]: a registry, and the grants its calls carry.
//! - [`Source`]: version control.
//! - [`Pause`] and [`Heartbeat`]: the only time seams. Nothing here reads a clock or sleeps.
//! - [`kit`]: for each seam but [`Source`], the scenarios every implementation must pass.
//!
//! This crate is pure and synchronous. Every seam is `&mut self` and `Send` (a session runs
//! one call at a time) except [`Source`], whose answers are reads.

mod build;
mod error;
mod files;
mod grant;
pub mod kit;
mod process;
mod publisher;
mod source;
mod store;
mod time;

pub use build::{BuildSystem, Command, Invocation, Located, Shape, TestOutcome, Verb};
pub use error::{Backoff, SeamError};
pub use files::{DirEntry, FileKind, OutputFiles, check_path, entry_path, valid_rel};
pub use grant::{Grant, GrantRequest, GrantSource};
pub use process::{Exit, ProcessResult, ProcessRunner, ProcessSpec};
pub use publisher::{
    Blob, CommitAnswer, CommitUnit, ListScope, Listed, Publisher, StageAnswer, StageItem,
};
pub use source::{Source, SourceStamp};
pub use store::{ArtifactStore, EntryWriter};
pub use time::{Heartbeat, Pause, Stop};
