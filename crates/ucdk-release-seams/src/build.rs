// SPDX-License-Identifier: Apache-2.0
//! Builds and tests, run through the build system the machine names.

pub use ucdk_release::invocation::{Command, Invocation, Verb};
use ucdk_release_proto::v1::BuildSystemKind;

use crate::{EntryWriter, Heartbeat, SeamError};

/// What `OutputFiles::stat` said of an output; never what a build system's listing said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    File,
    Directory,
}

/// One output the build system's own listing named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    pub label: String,
    /// Absolute, in the build system's output tree.
    pub path: String,
    pub shape: Shape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestOutcome {
    /// True only when the build system exited 0 and its report shows at least one test run.
    pub passed: bool,
    /// The test results the report says ran.
    pub ran: u32,
    /// The last lines of the output.
    pub tail: String,
}

/// An implementation knows its kind's command line and output listing. It never decides what to
/// build, where or on what hardware, and it never writes into the store.
///
/// `build` and `test` take the heartbeat beside the invocation, which is the layering's input
/// and holds none: it is forwarded to the process runner.
pub trait BuildSystem: Send {
    fn kind(&self) -> BuildSystemKind;

    /// The argument vector and environment of `inv`, with no effect: the layering as a pure
    /// function.
    fn command(&self, inv: &Invocation<'_>) -> Result<Command, SeamError> {
        ucdk_release::invocation::command(inv).map_err(|e| SeamError::Permanent(e.to_string()))
    }

    /// Runs the build, then lists its outputs with the build system's own query. A nonzero
    /// exit is `Permanent` with the tail of the output; a binary that cannot start is
    /// `Unknown`; a symlink in an output is `Refused(BUILD_OUTPUT)`. A target has exactly one
    /// output: the seam refuses a listing of none, or of two or more, as
    /// `Refused(BUILD_OUTPUT)`, so a successful build returns exactly one `Located`. The shape
    /// the artifact's type wants is the engine's to check against `Located::shape`.
    fn build(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<Vec<Located>, SeamError>;

    fn test(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<TestOutcome, SeamError>;

    /// Writes the files of one located output, and only those, into the entry it is given.
    fn copy_out(&mut self, out: &Located, into: &mut dyn EntryWriter) -> Result<(), SeamError>;
}
