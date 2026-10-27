// SPDX-License-Identifier: Apache-2.0
//! Starting a child from an argument vector and an explicit environment.

use ucdk_release::invocation::Command;

use crate::{Heartbeat, SeamError};

/// How a child ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
}

pub struct ProcessSpec<'a> {
    pub command: &'a Command,
    pub cwd: &'a str,
    pub heartbeat: &'a dyn Heartbeat,
    /// The most standard output to keep; over it, the tail is kept.
    pub stdout_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessResult {
    pub exit: Exit,
    pub stdout: Vec<u8>,
    pub stderr_tail: Vec<u8>,
}

/// Argument vector and an explicit environment only: no shell, nothing inherited. A bare
/// program name is looked for on the `PATH` of that environment and never on the runner's own.
pub trait ProcessRunner: Send {
    /// Beats on each chunk of output and each exit poll. `Stop` kills the child and its process
    /// group and answers `Refused(LEASE_LOST)`; a program that cannot be started is `Unknown`.
    fn run(&mut self, spec: &ProcessSpec<'_>) -> Result<ProcessResult, SeamError>;
}
