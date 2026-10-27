// SPDX-License-Identifier: Apache-2.0
//! The process-runner rig: the scripted fake with the behaviours of installed programs, and the
//! stubs that break one promise of a runner. The fake runs no process, so a child and its group
//! are two names in a set that the rig adds when a run starts and removes when it ends.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ucdk_core::ManualClock;
use ucdk_release::invocation::Command;
use ucdk_release_fake::FakeProcessRunner;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::kit::{Behavior, ProcessRig};
use ucdk_release_seams::{Exit, ProcessResult, ProcessRunner, ProcessSpec, SeamError};

pub const SCRATCH: &str = "/scratch";
pub const OWN_PATH: &str = "/runner/bin";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// A bare name is looked for on the runner's own `PATH`.
    ResolvesOnItsOwnPath,
    /// A bare name found nowhere is `Permanent`.
    BareNameNotFoundIsPermanent,
    /// The child sees a variable of the runner's environment.
    InheritsTheEnvironment,
    /// A shell expands `$HOME` in an argument.
    ShellExpandsAnArgument,
    /// A child ended by a signal is reported as the exit code 128 plus the signal.
    SignalIsAnExitCode,
    /// A program that cannot be started at an absolute path is an exit code 127.
    UnstartableIsAnExit,
    /// The head of the output is kept and not its tail.
    KeepsTheHead,
    /// A `Stop` is ignored and the child runs to its end.
    IgnoresStop,
    /// The process group of a stopped child is left running.
    LeavesTheGroupRunning,
    /// The child starts in the runner's own working directory, not the one it was given.
    CwdIgnored,
    /// A bare name is looked for on the runner's own `PATH` when the child's environment names
    /// no `PATH`.
    NoPathFallsBackToOwn,
    /// After a run was stopped, every later run of the runner is refused as stopped.
    AStopPoisonsTheRunner,
    /// The child sees the variables the previous run of the runner was given.
    EnvironmentLeaksIntoTheNextRun,
    /// When a bare name is on several directories of the child's `PATH`, the last one wins.
    LastPathEntryWins,
    /// Every later run starts in the working directory of the first.
    CwdIsFirstRuns,
    /// Every later run keeps the standard output limit of the first.
    LimitIsFirstRuns,
    /// A bare name that was found nowhere once is found nowhere again, whatever the `PATH`.
    PathMissCached,
    /// The tail of the error output is what every run so far wrote.
    StderrTailAccumulates,
    /// The child sees the variables an earlier run was given when that run could not be started.
    EnvLeaksAfterAnUnstartableRun,
    /// The child sees the variables an earlier run was given when that run was stopped.
    EnvLeaksAfterAStop,
    /// A child ended by a signal other than 9 is reported as the exit code 128 plus the signal.
    SignalOtherThan9IsCode128Plus,
    /// An exit code above 128 is reported as the signal that many above 128.
    HighCodeIsASignal,
    /// A limit of 0 keeps the whole of the output.
    ZeroLimitMeansUnlimited,
    /// An environment value is cut at its first `=`.
    EnvValueWithEqualsTruncated,
    /// A bare name is unstartable when a directory of the `PATH` does not exist.
    NonexistentPathEntryIsFatal,
}

pub struct FakeProcessRig {
    clock: ManualClock,
    flaw: Option<Flaw>,
    alive: Arc<Mutex<BTreeSet<String>>>,
}

impl FakeProcessRig {
    pub fn conforming() -> Self {
        Self {
            clock: ManualClock::at(1_000),
            flaw: None,
            alive: Arc::default(),
        }
    }

    pub fn flawed(flaw: Flaw) -> Self {
        Self {
            flaw: Some(flaw),
            ..Self::conforming()
        }
    }
}

pub struct Scripted {
    inner: FakeProcessRunner,
    flaw: Option<Flaw>,
    programs: BTreeMap<String, Behavior>,
    alive: Arc<Mutex<BTreeSet<String>>>,
    /// Whether a run of this runner was stopped.
    stopped: bool,
    /// The environment of the previous run.
    previous_env: Vec<(String, String)>,
    /// The working directory and the limit of the first run.
    first: Option<(String, usize)>,
    /// The bare names that were found nowhere.
    missed: Vec<String>,
    /// Every error output so far.
    stderr_seen: Vec<u8>,
    /// The environment of the last run that failed in the way the stub leaks after.
    leftover_env: Vec<(String, String)>,
}

impl ProcessRig for FakeProcessRig {
    type Runner = Scripted;

    fn runner(&self) -> Scripted {
        Scripted {
            inner: FakeProcessRunner::new(&self.clock),
            flaw: self.flaw,
            programs: BTreeMap::new(),
            alive: Arc::clone(&self.alive),
            stopped: false,
            previous_env: Vec::new(),
            first: None,
            missed: Vec::new(),
            stderr_seen: Vec::new(),
            leftover_env: Vec::new(),
        }
    }

    fn scratch(&self) -> String {
        SCRATCH.to_owned()
    }

    fn own_path_dir(&self) -> String {
        OWN_PATH.to_owned()
    }

    fn install(&self, runner: &mut Scripted, path: &str, behavior: Behavior) {
        runner.inner.program(path);
        runner.programs.insert(path.to_owned(), behavior);
    }

    fn alive(&self, process: &str) -> bool {
        self.alive
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(process)
    }
}

impl Scripted {
    fn is(&self, flaw: Flaw) -> bool {
        self.flaw == Some(flaw)
    }

    /// The installed program the command's first element names, on its own `PATH` when bare.
    fn program_of(&self, command: &Command) -> Option<&Behavior> {
        let name = command.argv.first()?;
        if name.starts_with('/') {
            return self.programs.get(name);
        }
        let dirs: Vec<&str> = command
            .env
            .iter()
            .filter(|(key, _)| key == "PATH")
            .flat_map(|(_, path)| path.split(':'))
            .collect();
        let found = |dir: &&str| self.programs.get(&format!("{dir}/{name}"));
        if self.is(Flaw::LastPathEntryWins) {
            dirs.iter().rev().find_map(found)
        } else {
            dirs.iter().find_map(found)
        }
    }

    /// Whether the directory `dir` exists: something was installed below it.
    fn dir_exists(&self, dir: &str) -> bool {
        dir == SCRATCH
            || dir == OWN_PATH
            || self
                .programs
                .keys()
                .any(|program| program.starts_with(&format!("{dir}/")))
    }

    fn set_alive(&self, names: &[String], alive: bool) {
        let mut set = self.alive.lock().unwrap_or_else(PoisonError::into_inner);
        for name in names {
            if alive {
                set.insert(name.clone());
            } else {
                set.remove(name);
            }
        }
    }
}

/// What a program that reports writes: its arguments, then its environment.
fn report(command: &Command, cwd: &str, flaw: Option<Flaw>) -> Vec<u8> {
    let cwd = if flaw == Some(Flaw::CwdIgnored) {
        "/runner"
    } else {
        cwd
    };
    let mut lines: Vec<String> = vec![format!("cwd {cwd}")];
    lines.extend(command.argv.iter().skip(1).map(|arg| {
        let arg = if flaw == Some(Flaw::ShellExpandsAnArgument) {
            arg.replace("$HOME", "/root")
        } else {
            arg.clone()
        };
        format!("arg {arg}")
    }));
    let mut env = command.env.clone();
    if flaw == Some(Flaw::InheritsTheEnvironment) {
        env.push(("HOME".to_owned(), "/root".to_owned()));
        env.sort();
    }
    lines.extend(env.iter().map(|(k, v)| format!("env {k}={v}")));
    let mut text = lines.join("\n");
    text.push('\n');
    text.into_bytes()
}

impl ProcessRunner for Scripted {
    fn run(&mut self, spec: &ProcessSpec<'_>) -> Result<ProcessResult, SeamError> {
        if self.is(Flaw::AStopPoisonsTheRunner) && self.stopped {
            return Err(SeamError::Refused(Reason::LeaseLost));
        }
        let mut command = spec.command.clone();
        let first = self
            .first
            .get_or_insert_with(|| (spec.cwd.to_owned(), spec.stdout_limit))
            .clone();
        let cwd = if self.is(Flaw::CwdIsFirstRuns) {
            first.0.as_str()
        } else {
            spec.cwd
        };
        let mut limit = if self.is(Flaw::LimitIsFirstRuns) {
            first.1
        } else {
            spec.stdout_limit
        };
        if self.is(Flaw::ZeroLimitMeansUnlimited) && limit == 0 {
            limit = usize::MAX;
        }
        if self.is(Flaw::EnvValueWithEqualsTruncated) {
            for (_, value) in &mut command.env {
                if let Some((head, _)) = value.split_once('=') {
                    *value = head.to_owned();
                }
            }
        }
        if matches!(
            self.flaw,
            Some(Flaw::EnvLeaksAfterAnUnstartableRun | Flaw::EnvLeaksAfterAStop)
        ) && !self.leftover_env.is_empty()
        {
            for (key, value) in std::mem::take(&mut self.leftover_env) {
                if !command.env.iter().any(|(k, _)| *k == key) {
                    command.env.push((key, value));
                }
            }
            command.env.sort();
        }
        let given_env = command.env.clone();
        let bare_name = command.argv.first().cloned().unwrap_or_default();
        if self.is(Flaw::PathMissCached) && self.missed.contains(&bare_name) {
            return Err(SeamError::Unknown(
                "the program cannot be started".to_owned(),
            ));
        }
        if self.is(Flaw::NonexistentPathEntryIsFatal)
            && !bare_name.starts_with('/')
            && command
                .env
                .iter()
                .filter(|(key, _)| key == "PATH")
                .flat_map(|(_, path)| path.split(':'))
                .any(|dir| !self.dir_exists(dir))
        {
            return Err(SeamError::Unknown(
                "the program cannot be started".to_owned(),
            ));
        }
        if self.is(Flaw::EnvironmentLeaksIntoTheNextRun) {
            let given = command.env.clone();
            for (key, value) in &self.previous_env {
                if !command.env.iter().any(|(k, _)| k == key) {
                    command.env.push((key.clone(), value.clone()));
                }
            }
            command.env.sort();
            self.previous_env = given;
        }
        let bare = !command
            .argv
            .first()
            .is_some_and(|name| name.starts_with('/'));
        let has_path = command.env.iter().any(|(key, _)| key == "PATH");
        let falls_back = self.is(Flaw::ResolvesOnItsOwnPath) && has_path
            || self.is(Flaw::NoPathFallsBackToOwn) && !has_path;
        if falls_back && bare {
            command.env.retain(|(key, _)| key != "PATH");
            command.env.push(("PATH".to_owned(), OWN_PATH.to_owned()));
        }
        let behavior = self.program_of(&command).cloned();
        let (exit, stdout, stderr) = match &behavior {
            Some(Behavior::Prints {
                stdout,
                stderr,
                code,
            }) => (Exit::Code(*code), stdout.clone(), stderr.clone()),
            Some(Behavior::Killed(signal)) => (Exit::Signal(*signal), Vec::new(), Vec::new()),
            Some(Behavior::Reports) => {
                (Exit::Code(0), report(&command, cwd, self.flaw), Vec::new())
            }
            Some(Behavior::Chatters { chunks }) => (
                Exit::Code(0),
                b"chunk\n".repeat(*chunks as usize),
                Vec::new(),
            ),
            Some(Behavior::Holds { .. }) | None => (Exit::Code(0), Vec::new(), Vec::new()),
        };
        let beats = match &behavior {
            Some(Behavior::Chatters { chunks }) => *chunks,
            Some(Behavior::Holds { .. }) => 3,
            _ => 0,
        };
        self.inner.beats(beats, Duration::from_millis(1));
        self.inner.script(command.clone(), exit, &stdout, &stderr);
        let held: Vec<String> = match &behavior {
            Some(Behavior::Holds { name }) => vec![name.clone(), format!("{name}.group")],
            _ => Vec::new(),
        };
        self.set_alive(&held, true);
        let mut result = self.inner.run(&ProcessSpec {
            command: &command,
            cwd,
            stdout_limit: limit,
            ..*spec
        });
        let stopped = result == Err(SeamError::Refused(Reason::LeaseLost));
        self.stopped |= stopped;
        let unstartable = matches!(result, Err(SeamError::Unknown(_)));
        if (self.is(Flaw::EnvLeaksAfterAStop) && stopped)
            || (self.is(Flaw::EnvLeaksAfterAnUnstartableRun) && unstartable)
        {
            self.leftover_env = given_env;
        }
        if unstartable && bare {
            self.missed.push(bare_name);
        }
        if let Ok(done) = &mut result
            && self.is(Flaw::StderrTailAccumulates)
        {
            self.stderr_seen.extend_from_slice(&done.stderr_tail);
            done.stderr_tail.clone_from(&self.stderr_seen);
        }
        // A child ends when the run does: it was killed on a Stop, or it came to its end. The
        // stub that leaves the group running forgets the grandchild of a stopped child.
        let survivors: Vec<String> = if self.is(Flaw::LeavesTheGroupRunning) && stopped {
            held.iter()
                .filter(|name| name.ends_with(".group"))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };
        let ended: Vec<String> = held
            .into_iter()
            .filter(|n| !survivors.contains(n))
            .collect();
        self.set_alive(&ended, false);
        match (self.flaw, &mut result) {
            (Some(Flaw::SignalIsAnExitCode), Ok(done)) => {
                if let Exit::Signal(n) = done.exit {
                    done.exit = Exit::Code(128 + n);
                }
            }
            (Some(Flaw::SignalOtherThan9IsCode128Plus), Ok(done)) => {
                if let Exit::Signal(n) = done.exit
                    && n != 9
                {
                    done.exit = Exit::Code(128 + n);
                }
            }
            (Some(Flaw::HighCodeIsASignal), Ok(done)) => {
                if let Exit::Code(c) = done.exit
                    && c > 128
                {
                    done.exit = Exit::Signal(c - 128);
                }
            }
            (Some(Flaw::UnstartableIsAnExit), Err(SeamError::Unknown(_))) if !bare => {
                result = Ok(ProcessResult {
                    exit: Exit::Code(127),
                    stdout: Vec::new(),
                    stderr_tail: Vec::new(),
                });
            }
            (Some(Flaw::BareNameNotFoundIsPermanent), Err(SeamError::Unknown(why))) if bare => {
                result = Err(SeamError::Permanent(why.clone()));
            }
            (Some(Flaw::KeepsTheHead), Ok(done)) => {
                done.stdout = stdout[..stdout.len().min(limit)].to_vec();
            }
            (Some(Flaw::IgnoresStop), Err(SeamError::Refused(Reason::LeaseLost))) => {
                result = Ok(ProcessResult {
                    exit: Exit::Code(0),
                    stdout,
                    stderr_tail: stderr,
                });
            }
            _ => {}
        }
        result
    }
}
