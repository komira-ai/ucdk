// SPDX-License-Identifier: Apache-2.0
//! Child processes, started from an argument vector and an explicit environment.

use ucdk_release_seams::{ProcessResult, ProcessRunner, ProcessSpec, SeamError};

/// A [`ProcessRunner`] over the child processes of this operating system.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsProcessRunner;

impl ProcessRunner for OsProcessRunner {
    fn run(&mut self, _spec: &ProcessSpec<'_>) -> Result<ProcessResult, SeamError> {
        Err(SeamError::Permanent("not written yet".to_owned()))
    }
}
