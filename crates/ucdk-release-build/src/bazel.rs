// SPDX-License-Identifier: Apache-2.0
//! The Bazel build system.

use ucdk_release::invocation::{Invocation, Verb};
use ucdk_release::listing;
use ucdk_release_proto::v1::BuildSystemKind;
use ucdk_release_seams::{
    BuildSystem, EntryWriter, Heartbeat, Located, OutputFiles, ProcessRunner, SeamError,
    TestOutcome,
};

use crate::shared::Shared;

/// Bazel over a process runner and a reader of the files it wrote.
///
/// A build is `bazel build`, and then two calls that find its output: `bazel cquery
/// --output=files`, which prints the output files of the target relative to the execution root,
/// and `bazel info execution_root`, which prints the root; both take the flags of the build, and
/// the path of the output is the root joined with the relative path. A test is one `bazel test`
/// that writes its build event file to the report file of the invocation. The command lines are
/// the layering of the design, which [`BuildSystem::command`] gives.
///
/// The output has to be on the local disk when the invocation returns. A Bazel that executes
/// remotely has to be told by the machine's own flags to download it (`--remote_download_toplevel`);
/// an output that is not there is `BUILD_OUTPUT`.
pub struct Bazel<R, F> {
    shared: Shared<R, F>,
}

impl<R: ProcessRunner, F: OutputFiles> Bazel<R, F> {
    /// Bazel that runs its invocations through `runner`, in the directory `root`, and reads the
    /// disk through `files`.
    pub fn new(runner: R, files: F, root: impl Into<String>) -> Self {
        Self {
            shared: Shared::new(runner, files, root.into()),
        }
    }
}

impl<R: ProcessRunner, F: OutputFiles> BuildSystem for Bazel<R, F> {
    fn kind(&self) -> BuildSystemKind {
        BuildSystemKind::Bazel
    }

    fn build(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<Vec<Located>, SeamError> {
        self.shared.succeed(inv, Verb::Build, heartbeat)?;
        let query = self.shared.succeed(inv, Verb::Query, heartbeat)?;
        let info = self.shared.succeed(inv, Verb::Info, heartbeat)?;
        let label = &inv.target.label;
        let output = listing::bazel(label, &query, &info)
            .map_err(|error| SeamError::Refused(error.reason()))?;
        self.shared.locate(label, &output.path)
    }

    fn test(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<TestOutcome, SeamError> {
        self.shared.test(BuildSystemKind::Bazel, inv, heartbeat)
    }

    fn copy_out(&mut self, out: &Located, into: &mut dyn EntryWriter) -> Result<(), SeamError> {
        self.shared.copy_out(out, into)
    }
}
