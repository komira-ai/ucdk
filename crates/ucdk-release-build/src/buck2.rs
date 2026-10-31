// SPDX-License-Identifier: Apache-2.0
//! The Buck2 build system.

use ucdk_release::invocation::Invocation;
use ucdk_release_proto::v1::BuildSystemKind;
use ucdk_release_seams::{
    BuildSystem, EntryWriter, Heartbeat, Located, OutputFiles, ProcessRunner, SeamError,
    TestOutcome,
};

/// Buck2 over a process runner and a reader of the files it wrote.
pub struct Buck2<R, F> {
    runner: R,
    files: F,
    root: String,
}

impl<R: ProcessRunner, F: OutputFiles> Buck2<R, F> {
    /// Buck2 that runs its invocations through `runner`, in the directory `root`, and reads the
    /// disk through `files`.
    pub fn new(runner: R, files: F, root: impl Into<String>) -> Self {
        Self {
            runner,
            files,
            root: root.into(),
        }
    }
}

fn not_written() -> SeamError {
    SeamError::Permanent("not written yet".to_owned())
}

impl<R: ProcessRunner, F: OutputFiles> BuildSystem for Buck2<R, F> {
    fn kind(&self) -> BuildSystemKind {
        BuildSystemKind::Buck2
    }

    fn build(
        &mut self,
        _inv: &Invocation<'_>,
        _heartbeat: &dyn Heartbeat,
    ) -> Result<Vec<Located>, SeamError> {
        let _ = (&self.runner, &self.files, &self.root);
        Err(not_written())
    }

    fn test(
        &mut self,
        _inv: &Invocation<'_>,
        _heartbeat: &dyn Heartbeat,
    ) -> Result<TestOutcome, SeamError> {
        Err(not_written())
    }

    fn copy_out(&mut self, _out: &Located, _into: &mut dyn EntryWriter) -> Result<(), SeamError> {
        Err(not_written())
    }
}
