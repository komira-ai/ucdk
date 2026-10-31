// SPDX-License-Identifier: Apache-2.0
//! Child processes, started from an argument vector and an explicit environment.

use std::collections::VecDeque;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as Process, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};
use ucdk_release::invocation::Command;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Exit, ProcessResult, ProcessRunner, ProcessSpec, SeamError};

/// How long the runner waits for output before it polls for the end of the child and beats.
const POLL: Duration = Duration::from_millis(20);
/// The most a read of a pipe takes at once.
const CHUNK: usize = 8192;
/// The most of the child's error output that is kept: its tail.
const STDERR_KEPT: usize = 64 * 1024;
/// How often, and at most how many times, a stop looks whether the group has gone.
const GONE_NAP: Duration = Duration::from_millis(5);
const GONE_POLLS: usize = 200;

/// What the threads that read the child's pipes tell the runner.
enum Event {
    Out(Vec<u8>),
    Err(Vec<u8>),
    /// The pipe is at its end.
    Done,
}

/// A [`ProcessRunner`] over the child processes of this operating system.
///
/// The child is started from the argument vector and the environment of its command and from
/// nothing else: no shell reads an argument, no variable of this process reaches the child, and
/// a program named without a `/` is looked for on the `PATH` of the command's own environment
/// (a directory of it that is not absolute is relative to the working directory) and never on
/// this process's. The child leads a process group of its own. The runner beats on each chunk
/// of output and on each poll for the child's end, and a `Stop` kills the group and answers
/// `Refused(LEASE_LOST)`. It has no time limit: a build that never ends is ended by the
/// heartbeat, as every build is. The call returns once the child has ended and every holder of
/// its output has closed it, so a process the child left behind with its output open keeps the
/// call (and the beats) going until it ends or a `Stop` comes.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsProcessRunner;

/// Whether `path` is a regular file that someone may execute.
fn executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() & (m.permissions().mode() & 0o111 != 0))
}

/// The file that starts `program`, as `exec` finds it: a name with a `/` in it is relative to
/// `cwd`, and any other is looked for in order in the directories of the `PATH` of `env` (the
/// last variable of that name, as the child gets it), each relative to `cwd` when it is not
/// absolute. A candidate that is not an executable file is passed over.
fn find_program(program: &str, cwd: &str, env: &[(String, String)]) -> Option<PathBuf> {
    let base = Path::new(cwd);
    if program.contains('/') {
        return Some(base.join(program)).filter(|path| executable(path));
    }
    let (_, path) = env.iter().rev().find(|(key, _)| key == "PATH")?;
    path.split(':')
        .map(|dir| base.join(dir).join(program))
        .find(|candidate| executable(candidate))
}

/// Reads `pipe` to its end on a thread of its own and tells `events` of each chunk, wrapped as
/// `wrap` says, and then that it is done.
fn pump(mut pipe: impl Read + Send + 'static, events: Sender<Event>, wrap: fn(Vec<u8>) -> Event) {
    thread::spawn(move || {
        let mut buffer = [0_u8; CHUNK];
        loop {
            let read = pipe.read(&mut buffer).unwrap_or(0);
            if read == 0 {
                break;
            }
            // The runner listens until both pipes are done, so a failed send has no one to tell.
            let _ = events.send(wrap(buffer[..read].to_vec()));
        }
        let _ = events.send(Event::Done);
    });
}

/// Appends `chunk` to `kept` and drops the front of it to leave at most `limit` bytes.
fn keep_tail(kept: &mut VecDeque<u8>, chunk: &[u8], limit: usize) {
    kept.extend(chunk);
    let excess = kept.len().saturating_sub(limit);
    kept.drain(..excess);
}

/// Kills the child's whole process group, collects the child, and waits until the group has no
/// member left: the signal is delivered at once, and a member exits a moment after it. A member
/// that has ended and that nobody has collected yet is still a member, so the wait is for its
/// parent to collect it, and it is bounded.
fn stop(child: &mut Child) {
    let group = Pid::from_child(child);
    // The group may be gone already, and then there is nothing to kill.
    let _ = kill_process_group(group, Signal::KILL);
    let _ = child.wait();
    (0..GONE_POLLS)
        .take_while(|_| test_kill_process_group(group).is_ok())
        .for_each(|_| thread::sleep(GONE_NAP));
}

impl ProcessRunner for OsProcessRunner {
    fn run(&mut self, spec: &ProcessSpec<'_>) -> Result<ProcessResult, SeamError> {
        let Command { argv, env } = spec.command;
        let Some((program, args)) = argv.split_first() else {
            return Err(SeamError::Permanent(
                "the argument vector holds no program".to_owned(),
            ));
        };
        let Some(path) = find_program(program, spec.cwd, env) else {
            return Err(SeamError::Unknown(format!(
                "{program} is not an executable file where the child would look for it"
            )));
        };
        let mut child = Process::new(&path)
            .arg0(program)
            .args(args)
            .env_clear()
            .envs(env.iter().cloned())
            .current_dir(spec.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|error| {
                SeamError::Unknown(format!("{program} could not be started: {error}"))
            })?;
        let (events, heard) = mpsc::channel();
        let (out, err) = (child.stdout.take(), child.stderr.take());
        pump(out.expect("stdout is piped"), events.clone(), Event::Out);
        pump(err.expect("stderr is piped"), events.clone(), Event::Err);
        let (mut stdout, mut stderr) = (VecDeque::new(), VecDeque::new());
        let (mut open, mut ended) = (2_u8, None);
        // `events` stays alive here, so a wait for an event ends by its time and never because
        // the pipes are done: the loop polls for the child's end at that pace.
        let status = loop {
            match heard.recv_timeout(POLL) {
                Ok(Event::Out(bytes)) => keep_tail(&mut stdout, &bytes, spec.stdout_limit),
                Ok(Event::Err(bytes)) => keep_tail(&mut stderr, &bytes, STDERR_KEPT),
                Ok(Event::Done) => open -= 1,
                Err(_) => {}
            }
            if ended.is_none() {
                ended = child.try_wait().unwrap_or(None);
            }
            if spec.heartbeat.beat().is_err() {
                stop(&mut child);
                return Err(SeamError::Refused(Reason::LeaseLost));
            }
            if let (Some(status), 0) = (ended, open) {
                break status;
            }
        };
        Ok(ProcessResult {
            exit: status
                .code()
                .map_or_else(|| Exit::Signal(status.signal().unwrap_or(0)), Exit::Code),
            stdout: Vec::from(stdout),
            stderr_tail: Vec::from(stderr),
        })
    }
}
