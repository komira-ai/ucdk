// SPDX-License-Identifier: Apache-2.0
//! A process runner scripted by argument vector and environment, for the build systems and the
//! git source to run over. A call nobody scripted fails the test.

use std::collections::BTreeSet;
use std::time::Duration;

use ucdk_core::ManualClock;
use ucdk_release::invocation::Command;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Exit, ProcessResult, ProcessRunner, ProcessSpec, SeamError};

/// One call the runner was asked to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCall {
    pub command: Command,
    pub cwd: String,
}

pub struct FakeProcessRunner {
    clock: ManualClock,
    scripts: Vec<(Command, ProcessResult)>,
    programs: BTreeSet<String>,
    calls: Vec<RunCall>,
    die_at: Option<usize>,
    signal_at: Option<usize>,
    unstartable: bool,
    beats: Option<(u32, Duration)>,
    silent_for: Option<Duration>,
}

impl FakeProcessRunner {
    pub fn new(clock: &ManualClock) -> Self {
        Self {
            clock: clock.clone(),
            scripts: Vec::new(),
            programs: BTreeSet::new(),
            calls: Vec::new(),
            die_at: None,
            signal_at: None,
            unstartable: false,
            beats: None,
            silent_for: None,
        }
    }

    /// An executable exists at the absolute path `path`, so a bare name is found where it is
    /// and an absolute name is startable.
    pub fn program(&mut self, path: &str) {
        self.programs.insert(path.to_owned());
    }

    /// The exact `command` ends with `exit`, printing `stdout` and `stderr_tail`.
    pub fn script(&mut self, command: Command, exit: Exit, stdout: &[u8], stderr_tail: &[u8]) {
        self.scripts.push((
            command,
            ProcessResult {
                exit,
                stdout: stdout.to_vec(),
                stderr_tail: stderr_tail.to_vec(),
            },
        ));
    }

    /// The `n`th call (1 is the first) is a child that was killed under the runner: `Unknown`.
    pub fn die_at_call(&mut self, n: usize) {
        self.die_at = Some(n);
    }

    /// The `n`th call ends by a signal.
    pub fn signal(&mut self, n: usize) {
        self.signal_at = Some(n);
    }

    /// No program can be started.
    pub fn unstartable(&mut self) {
        self.unstartable = true;
    }

    /// Each call beats `n` times, moving the clock by `step` before each beat.
    pub fn beats(&mut self, n: u32, step: Duration) {
        self.beats = Some((n, step));
    }

    /// Each call moves the clock by `by` and never beats.
    pub fn silent_for(&mut self, by: Duration) {
        self.silent_for = Some(by);
    }

    /// Every call, in order, with its command and directory.
    pub fn calls(&self) -> &[RunCall] {
        &self.calls
    }

    /// Whether the program of `command` is found, as `exec` finds it: an absolute name where it
    /// is, a name with a `/` in it relative to the working directory `cwd`, and only a bare name
    /// on the `PATH` of the command's own environment, where a directory that is not absolute is
    /// relative to `cwd` too.
    fn startable(&self, command: &Command, cwd: &str) -> bool {
        let program = command.argv.first().map_or("", String::as_str);
        if program.starts_with('/') {
            return self.programs.contains(program);
        }
        if program.contains('/') {
            return self.programs.contains(&format!("{cwd}/{program}"));
        }
        command
            .env
            .iter()
            .filter(|(key, _)| key == "PATH")
            .flat_map(|(_, path)| path.split(':'))
            .any(|dir| {
                // A directory of the `PATH` that is not absolute is relative to `cwd`.
                let dir = if dir.starts_with('/') {
                    dir.to_owned()
                } else {
                    format!("{cwd}/{dir}")
                };
                self.programs.contains(&format!("{dir}/{program}"))
            })
    }
}

impl ProcessRunner for FakeProcessRunner {
    fn run(&mut self, spec: &ProcessSpec<'_>) -> Result<ProcessResult, SeamError> {
        self.calls.push(RunCall {
            command: spec.command.clone(),
            cwd: spec.cwd.to_owned(),
        });
        let n = self.calls.len();
        // A call nobody scripted fails the test, whether or not its program is installed.
        let Some((_, reply)) = self.scripts.iter().find(|(c, _)| c == spec.command) else {
            panic!("a call nobody scripted: {:?}", spec.command);
        };
        if self.unstartable || !self.startable(spec.command, spec.cwd) {
            return Err(SeamError::Unknown(
                "the program cannot be started".to_owned(),
            ));
        }
        if self.die_at == Some(n) {
            return Err(SeamError::Unknown("the child was killed".to_owned()));
        }
        let (beats, step) = self.beats.unwrap_or_default();
        for _ in 0..beats {
            self.clock.advance(step);
            spec.heartbeat
                .beat()
                .map_err(|_| SeamError::Refused(Reason::LeaseLost))?;
        }
        if let Some(by) = self.silent_for {
            self.clock.advance(by);
        }
        if self.signal_at == Some(n) {
            return Ok(ProcessResult {
                exit: Exit::Signal(9),
                stdout: Vec::new(),
                stderr_tail: Vec::new(),
            });
        }
        let keep = reply.stdout.len().saturating_sub(spec.stdout_limit);
        Ok(ProcessResult {
            stdout: reply.stdout[keep..].to_vec(),
            ..reply.clone()
        })
    }
}
