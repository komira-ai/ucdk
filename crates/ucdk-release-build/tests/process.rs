// SPDX-License-Identifier: Apache-2.0
//! The real process runner, held to the process-runner kit with the program of `tests/child`
//! installed under the names the scenarios use, and to the promises the kit does not make.
//!
//! The kit's scenarios about the `PATH` need a decoy on this process's own `PATH`, and a test
//! cannot change its own environment. So the test that holds them starts this test binary again
//! with such a `PATH`, and the whole kit runs there.

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command as Process;
use std::sync::{Mutex, PoisonError};

use support::{Never, hex, run_scenario};
use tempfile::TempDir;
use ucdk_release::invocation::Command;
use ucdk_release_build::OsProcessRunner;
use ucdk_release_seams::kit::{
    Behavior, ProcessRig, process_runner_scenarios, run_process_runner_kit,
};
use ucdk_release_seams::{Exit, ProcessResult, ProcessRunner, ProcessSpec, SeamError};

/// The program the tests install under every name.
const CHILD: &str = env!("CARGO_BIN_EXE_ucdk-test-child");
/// The kit names the child it holds `held`, so two runs of it cannot be in the process table at
/// once: the test of the kit in a process of its own and the one that holds a child here.
static ONE_HOLDER: Mutex<()> = Mutex::new(());
/// Set in the copy of this binary that runs the whole kit, to the directory on its own `PATH`.
const INNER: &str = "UCDK_PROCESS_KIT_OWN_PATH";

struct Rig {
    _dir: TempDir,
    scratch: String,
    own: String,
}

impl Rig {
    /// A rig whose decoy directory is `own` when it is given, and a directory of its own that
    /// is on no `PATH` otherwise.
    fn new(own: Option<String>) -> Self {
        let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("a directory");
        let scratch = fs::canonicalize(dir.path())
            .expect("its path")
            .to_string_lossy()
            .into_owned();
        let own = own.unwrap_or_else(|| format!("{scratch}/off-path"));
        Self {
            _dir: dir,
            scratch,
            own,
        }
    }
}

/// How the program of `tests/child` is told to behave: the lines of the file beside it.
fn behavior_file(behavior: &Behavior) -> String {
    match behavior {
        Behavior::Prints {
            stdout,
            stderr,
            code,
        } => format!("prints {code} {} {}\n", hex(stdout), hex(stderr)),
        Behavior::Killed(signal) => format!("killed {signal}\n"),
        Behavior::Reports => "reports\n".to_owned(),
        Behavior::Chatters { chunks } => format!("chatters {chunks}\n"),
        Behavior::Holds { name } => format!("holds {name}\n"),
    }
}

impl ProcessRig for Rig {
    type Runner = OsProcessRunner;

    fn runner(&self) -> OsProcessRunner {
        OsProcessRunner
    }

    fn scratch(&self) -> String {
        self.scratch.clone()
    }

    fn own_path_dir(&self) -> String {
        self.own.clone()
    }

    fn install(&self, _runner: &mut OsProcessRunner, path: &str, behavior: Behavior) {
        fs::create_dir_all(Path::new(path).parent().expect("a directory")).expect("directories");
        // A link and not a copy: nothing opens the program for writing, so a program started
        // in another test at the same moment cannot make this one "text file busy".
        let _ = fs::remove_file(path);
        fs::hard_link(CHILD, path)
            .or_else(|_| fs::copy(CHILD, path).map(|_| ()))
            .expect("the program is put in place");
        fs::write(format!("{path}.behavior"), behavior_file(&behavior)).expect("its behavior");
    }

    /// Whether a process called `process` is in the process table, a zombie of this test
    /// process included: a runner that does not collect the child it killed leaves one. A
    /// zombie of any other process is not this runner's to collect.
    fn alive(&self, process: &str) -> bool {
        let me = std::process::id().to_string();
        fs::read_dir("/proc")
            .expect("the process table")
            .flatten()
            .any(|entry| {
                let dir = entry.path();
                let Ok(comm) = fs::read_to_string(dir.join("comm")) else {
                    return false;
                };
                let Ok(stat) = fs::read_to_string(dir.join("stat")) else {
                    return false;
                };
                // `pid (comm) state ppid ...`, and the name may hold spaces and parentheses.
                let after = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
                let mut fields = after.split_whitespace();
                let (state, parent) = (fields.next(), fields.next());
                (comm.trim_end() == process)
                    & ((state != Some("Z")) | (parent == Some(me.as_str())))
            })
    }
}

fn scenario(name: &str) {
    run_scenario(&process_runner_scenarios(), name, &Rig::new(None));
}

fn command(argv: &[&str], env: &[(&str, &str)]) -> Command {
    Command {
        argv: argv.iter().map(|a| (*a).to_owned()).collect(),
        env: env
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
    }
}

fn run(command: &Command, cwd: &str) -> Result<ProcessResult, SeamError> {
    OsProcessRunner.run(&ProcessSpec {
        command,
        cwd,
        heartbeat: &Never,
        stdout_limit: 1 << 20,
    })
}

#[test]
fn argv_and_environment_reach_the_child_exactly() {
    scenario("argv_and_environment_are_exactly_what_was_given");
}

#[test]
fn an_argument_with_spaces_and_metacharacters_is_one_element() {
    scenario("no_shell_interprets_an_argument");
}

#[test]
fn the_environment_is_not_inherited() {
    let rig = Rig::new(None);
    let program = format!("{}/report", rig.scratch);
    rig.install(&mut OsProcessRunner, &program, Behavior::Reports);
    // This process has an environment, and the child sees none of it.
    assert!(std::env::vars_os().next().is_some());
    let result = run(&command(&[&program], &[]), &rig.scratch).unwrap();
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        format!("cwd {}\n", rig.scratch)
    );
}

#[test]
fn a_nonzero_exit_and_a_signal_are_told_apart() {
    scenario("exit_code_and_signal_are_distinguished");
}

#[test]
fn a_missing_executable_is_unstartable_not_a_nonzero_exit() {
    scenario("unstartable_is_not_a_nonzero_exit");
}

#[test]
fn output_over_the_limit_keeps_its_tail() {
    scenario("output_is_bounded_and_its_tail_is_kept");
    // The error output is bounded too, to its tail.
    let rig = Rig::new(None);
    let program = format!("{}/noisy", rig.scratch);
    let stderr: Vec<u8> = (0..200_000_u32).map(|i| (i % 251) as u8).collect();
    rig.install(
        &mut OsProcessRunner,
        &program,
        Behavior::Prints {
            stdout: Vec::new(),
            stderr: stderr.clone(),
            code: 0,
        },
    );
    let result = run(&command(&[&program], &[]), &rig.scratch).unwrap();
    assert_eq!(result.stderr_tail.len(), 64 * 1024);
    assert!(stderr.ends_with(&result.stderr_tail));
}

#[test]
fn beats_on_each_chunk_of_output() {
    scenario("the_runner_beats_on_progress_and_stops_the_child_when_told");
}

#[test]
fn a_child_and_its_group_are_killed_when_a_beat_says_stop() {
    let _alone = ONE_HOLDER.lock().unwrap_or_else(PoisonError::into_inner);
    scenario("a_stopped_child_and_its_process_group_are_gone");
}

#[test]
fn an_empty_argument_vector_has_no_program_to_start() {
    let rig = Rig::new(None);
    let refusal = run(&command(&[], &[]), &rig.scratch).unwrap_err();
    assert!(matches!(refusal, SeamError::Permanent(_)), "{refusal:?}");
}

#[test]
fn a_working_directory_that_is_not_there_is_unstartable() {
    let rig = Rig::new(None);
    let program = format!("{}/report", rig.scratch);
    rig.install(&mut OsProcessRunner, &program, Behavior::Reports);
    let nowhere = format!("{}/nowhere", rig.scratch);
    let refusal = run(&command(&[&program], &[]), &nowhere).unwrap_err();
    assert!(matches!(refusal, SeamError::Unknown(_)), "{refusal:?}");
}

#[test]
fn a_directory_and_a_file_that_is_not_executable_are_passed_over_on_the_path() {
    let rig = Rig::new(None);
    let s = &rig.scratch;
    // `tool` is a directory in the first directory of the PATH, a file nobody may execute in
    // the second, and the program in the third.
    fs::create_dir_all(format!("{s}/a/tool")).unwrap();
    fs::create_dir_all(format!("{s}/b")).unwrap();
    fs::write(format!("{s}/b/tool"), b"not a program").unwrap();
    fs::set_permissions(format!("{s}/b/tool"), fs::Permissions::from_mode(0o644)).unwrap();
    rig.install(
        &mut OsProcessRunner,
        &format!("{s}/c/tool"),
        Behavior::Reports,
    );
    let path = format!("{s}/a:{s}/b:{s}/c");
    let result = run(&command(&["tool"], &[("PATH", &path)]), s).unwrap();
    assert_eq!(result.exit, Exit::Code(0));
    // Not found, when only the first two are on the PATH.
    let path = format!("{s}/a:{s}/b");
    let refusal = run(&command(&["tool"], &[("PATH", &path)]), s).unwrap_err();
    assert!(matches!(refusal, SeamError::Unknown(_)), "{refusal:?}");
}

#[test]
fn the_last_path_variable_of_the_environment_is_the_path() {
    let rig = Rig::new(None);
    let s = &rig.scratch;
    rig.install(
        &mut OsProcessRunner,
        &format!("{s}/second/tool"),
        Behavior::Reports,
    );
    let second = format!("{s}/second");
    let env = [("PATH", "/nowhere"), ("PATH", second.as_str())];
    let result = run(&command(&["tool"], &env), s).unwrap();
    // The child has one variable of that name, the last, as the lookup used.
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains(&format!("env PATH={s}/second\n"))
    );
}

#[test]
fn the_kit_passes_with_a_decoy_on_the_test_processs_own_path() {
    if let Ok(own) = std::env::var(INNER) {
        // The copy of this binary that the test below started: the kit runs here, where the
        // directory `own` is on the PATH of this very process.
        run_process_runner_kit(&Rig::new(Some(own))).assert_conforms();
        return;
    }
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("a directory");
    let own = fs::canonicalize(dir.path())
        .expect("its path")
        .join("own-bin");
    fs::create_dir_all(&own).expect("the decoy directory");
    let _alone = ONE_HOLDER.lock().unwrap_or_else(PoisonError::into_inner);
    let exe = std::env::current_exe().expect("this binary");
    let output = Process::new(exe)
        .args([
            "--exact",
            "the_kit_passes_with_a_decoy_on_the_test_processs_own_path",
            "--test-threads=1",
            "--nocapture",
        ])
        .env(INNER, &own)
        .env("PATH", &own)
        .output()
        .expect("this binary starts again");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    // It ran the test it was asked to, and not none.
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}
