// SPDX-License-Identifier: Apache-2.0
//! The real build side of a release: a process runner over child processes, a reader of the
//! local files a build wrote, and the Buck2 and Bazel build systems over them.

mod bazel;
mod buck2;
mod files;
mod process;

pub use bazel::Bazel;
pub use buck2::Buck2;
pub use files::LocalFiles;
pub use process::OsProcessRunner;
