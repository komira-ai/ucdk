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
            Some(Behavior::Reports) => (
                Exit::Code(0),
                report(&command, spec.cwd, self.flaw),
                Vec::new(),
            ),
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
            ..*spec
        });
        let stopped = result == Err(SeamError::Refused(Reason::LeaseLost));
        self.stopped |= stopped;
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
                done.stdout = stdout[..stdout.len().min(spec.stdout_limit)].to_vec();
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
