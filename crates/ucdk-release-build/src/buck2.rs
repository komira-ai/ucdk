// SPDX-License-Identifier: Apache-2.0
//! The Buck2 build system.

use ucdk_release::invocation::{Invocation, Verb};
use ucdk_release::listing;
use ucdk_release_proto::v1::BuildSystemKind;
use ucdk_release_seams::{
    BuildSystem, EntryWriter, Heartbeat, Located, OutputFiles, ProcessRunner, SeamError,
    TestOutcome,
};

use crate::shared::Shared;

/// Buck2 over a process runner and a reader of the files it wrote.
///
/// A build is one `buck2 build --show-full-output <label>` in `root`, and its output is the one
/// the invocation lists on standard output: the label, a space and the absolute path (the path
/// may hold spaces, so a line is split at the first space). A test is one `buck2 test` that
/// writes its event log to the report file of the invocation. The command line is the layering
/// of the design, which [`BuildSystem::command`] gives.
pub struct Buck2<R, F> {
    shared: Shared<R, F>,
}

impl<R: ProcessRunner, F: OutputFiles> Buck2<R, F> {
    /// Buck2 that runs its invocations through `runner`, in the directory `root`, and reads the
    /// disk through `files`.
    pub fn new(runner: R, files: F, root: impl Into<String>) -> Self {
        Self {
            shared: Shared::new(runner, files, root.into()),
        }
    }
}

impl<R: ProcessRunner, F: OutputFiles> BuildSystem for Buck2<R, F> {
    fn kind(&self) -> BuildSystemKind {
        BuildSystemKind::Buck2
    }

    fn build(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<Vec<Located>, SeamError> {
        let listing = self.shared.succeed(inv, Verb::Build, heartbeat)?;
        let label = &inv.target.label;
        let output =
            listing::buck2(label, &listing).map_err(|error| SeamError::Refused(error.reason()))?;
        self.shared.locate(label, &output.path)
    }

    fn test(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<TestOutcome, SeamError> {
        self.shared.test(BuildSystemKind::Buck2, inv, heartbeat)
    }

    fn copy_out(&mut self, out: &Located, into: &mut dyn EntryWriter) -> Result<(), SeamError> {
        self.shared.copy_out(out, into)
    }
}
