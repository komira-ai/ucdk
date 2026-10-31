// SPDX-License-Identifier: Apache-2.0
//! The real build side of a release: what the engine calls when it builds and tests with a
//! build system on this machine. Each type here is held to the kit of its seam, which the fakes
//! of `ucdk-release-fake` are held to as well.
//!
//! - [`OsProcessRunner`]: a [`ProcessRunner`](ucdk_release_seams::ProcessRunner) over child
//!   processes. The child gets the argument vector and the environment it was given and nothing
//!   else, in a process group of its own that a stop kills as a whole.
//! - [`LocalFiles`]: an [`OutputFiles`](ucdk_release_seams::OutputFiles) over the local disk.
//!   It only reads, and never follows a symlink.
//! - [`Buck2`] and [`Bazel`]: [`BuildSystem`](ucdk_release_seams::BuildSystem)s over any process
//!   runner and any reader of output files, so the tests of their command lines and of the
//!   output they find run over the scripted fakes and never start a build tool.

mod bazel;
mod buck2;
mod files;
mod process;
mod shared;

pub use bazel::Bazel;
pub use buck2::Buck2;
pub use files::LocalFiles;
pub use process::OsProcessRunner;
