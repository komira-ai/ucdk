// SPDX-License-Identifier: Apache-2.0
//! The process runner kit: nine scenarios over starting a child from an argument vector and an
//! explicit environment, and over ending it.

use std::cell::Cell;

use ucdk_release::invocation::Command;
use ucdk_release_proto::v1::Reason;

use super::{Beats, Report, Scenario, must, must_err, run_scenarios, same};
use crate::{Exit, ProcessResult, ProcessRunner, ProcessSpec, SeamError};

/// What an installed program does when it is run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Behavior {
    /// Writes `stdout` and `stderr`, then ends with the exit code `code`.
    Prints {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        code: i32,
    },
    /// Ends by the signal `n`.
    Killed(i32),
    /// Writes one line `cwd <dir>` with its working directory as the system reports it, then
    /// one line `arg <text>` for each argument after its own name, then one line
    /// `env <key>=<value>` for each variable of its environment, sorted by key, then ends with
    /// code 0.
    Reports,
    /// Writes `chunks` chunks of output one after the other, then ends with code 0.
    Chatters { chunks: u32 },
    /// Starts a grandchild in its own process group, then waits until it is killed. The child is
    /// alive as `name` and the grandchild as `<name>.group` for as long as they run.
    Holds { name: String },
}

/// What an implementation provides: a runner, and programs for it to run.
pub trait ProcessRig {
    type Runner: ProcessRunner;

    fn runner(&self) -> Self::Runner;

    /// An absolute path of an existing directory that scenarios may run in and install under.
    /// No part of it is a symlink, so that a child reports it as it is given here.
    fn scratch(&self) -> String;

    /// A directory on the runner's own `PATH`, where a decoy can be installed.
    fn own_path_dir(&self) -> String;

    /// An executable at the absolute path `path` (its directories made) that does `behavior`
    /// when it is run, whatever the argument vector.
    fn install(&self, runner: &mut Self::Runner, path: &str, behavior: Behavior);

    /// Whether the child or grandchild called `process` (see [`Behavior::Holds`]) is running.
    fn alive(&self, process: &str) -> bool;
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

/// Runs `command` in the scratch directory, with a heartbeat that never stops and no real bound
/// on the output.
fn run_plain<R: ProcessRig>(
    rig: &R,
    runner: &mut R::Runner,
    command: &Command,
) -> Result<ProcessResult, SeamError> {
    run_with(runner, command, &rig.scratch(), 1 << 20)
}

/// Runs `command` in `cwd`, with a heartbeat that never stops and the given bound on the output.
fn run_with<N: ProcessRunner>(
    runner: &mut N,
    command: &Command,
    cwd: &str,
    stdout_limit: usize,
) -> Result<ProcessResult, SeamError> {
    let beats = Beats::counting();
    runner.run(&ProcessSpec {
        command,
        cwd,
        heartbeat: &beats,
        stdout_limit,
    })
}

fn exits(code: i32) -> Behavior {
    Behavior::Prints {
        stdout: Vec::new(),
        stderr: Vec::new(),
        code,
    }
}

fn prints(stdout: &str) -> Behavior {
    Behavior::Prints {
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
        code: 0,
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

pub fn run_process_runner_kit<R: ProcessRig>(rig: &R) -> Report {
    run_scenarios(rig, &process_runner_scenarios())
}

pub fn process_runner_scenarios<R: ProcessRig>() -> Vec<Scenario<R>> {
    vec![
        Scenario::new(
            "a_bare_name_is_resolved_on_the_childs_path_and_never_the_runners",
            a_bare_name_is_resolved_on_the_childs_path_and_never_the_runners::<R>,
        ),
        Scenario::new(
            "a_name_found_nowhere_on_the_childs_path_is_unstartable",
            a_name_found_nowhere_on_the_childs_path_is_unstartable::<R>,
        ),
        Scenario::new(
            "argv_and_environment_are_exactly_what_was_given",
            argv_and_environment_are_exactly_what_was_given::<R>,
        ),
        Scenario::new(
            "no_shell_interprets_an_argument",
            no_shell_interprets_an_argument::<R>,
        ),
        Scenario::new(
            "exit_code_and_signal_are_distinguished",
            exit_code_and_signal_are_distinguished::<R>,
        ),
        Scenario::new(
            "unstartable_is_not_a_nonzero_exit",
            unstartable_is_not_a_nonzero_exit::<R>,
        ),
        Scenario::new(
            "output_is_bounded_and_its_tail_is_kept",
            output_is_bounded_and_its_tail_is_kept::<R>,
        ),
        Scenario::new(
            "the_runner_beats_on_progress_and_stops_the_child_when_told",
            the_runner_beats_on_progress_and_stops_the_child_when_told::<R>,
        ),
        Scenario::new(
            "a_stopped_child_and_its_process_group_are_gone",
            a_stopped_child_and_its_process_group_are_gone::<R>,
        ),
    ]
}

fn a_bare_name_is_resolved_on_the_childs_path_and_never_the_runners<R: ProcessRig>(rig: &R) {
    let mut runner = rig.runner();
    let child_dir = format!("{}/child-bin", rig.scratch());
    let other_dir = format!("{}/other-bin", rig.scratch());
    rig.install(&mut runner, &format!("{child_dir}/tool"), prints("child"));
    rig.install(&mut runner, &format!("{other_dir}/tool"), prints("other"));
    rig.install(
        &mut runner,
        &format!("{}/tool", rig.own_path_dir()),
        prints("decoy"),
    );
    let result = must(
        run_plain(
            rig,
            &mut runner,
            &command(&["tool"], &[("PATH", &child_dir)]),
        ),
        "run of a bare name",
    );
    same(
        text(&result.stdout),
        "child".to_owned(),
        "the program that ran",
    );
    // With the name in two directories of the `PATH`, the first of them in the order given wins.
    for (path, want) in [
        (format!("{child_dir}:{other_dir}"), "child"),
        (format!("{other_dir}:{child_dir}"), "other"),
    ] {
        let result = must(
            run_plain(rig, &mut runner, &command(&["tool"], &[("PATH", &path)])),
            "run of a bare name found in two directories",
        );
        same(
            text(&result.stdout),
            want.to_owned(),
            &format!("the program that ran with PATH {path}"),
        );
    }
    // A directory of the `PATH` that is not there is skipped, not fatal.
    let missing = format!("{}/missing-bin", rig.scratch());
    let path = format!("{missing}:{child_dir}");
    let result = must(
        run_plain(rig, &mut runner, &command(&["tool"], &[("PATH", &path)])),
        "run of a bare name with a directory on the PATH that does not exist",
    );
    same(
        text(&result.stdout),
        "child".to_owned(),
        "the program that ran with a directory of the PATH that does not exist",
    );
    // A name that was not found on one `PATH` is found on another that has it.
    let empty = format!("{}/empty-bin", rig.scratch());
    rig.install(&mut runner, &format!("{empty}/other-tool"), prints("x"));
    must_err(
        run_plain(rig, &mut runner, &command(&["tool"], &[("PATH", &empty)])),
        "run of a bare name that the PATH does not have",
    );
    let result = must(
        run_plain(
            rig,
            &mut runner,
            &command(&["tool"], &[("PATH", &child_dir)]),
        ),
        "run of a bare name found after it was missed",
    );
    same(
        text(&result.stdout),
        "child".to_owned(),
        "the program that ran after the name was missed on another PATH",
    );
}

fn a_name_found_nowhere_on_the_childs_path_is_unstartable<R: ProcessRig>(rig: &R) {
    let mut runner = rig.runner();
    let child_dir = format!("{}/child-bin", rig.scratch());
    let error = must_err(
        run_plain(
            rig,
            &mut runner,
            &command(&["absent"], &[("PATH", &child_dir)]),
        ),
        "run of a bare name found nowhere",
    );
    same(
        matches!(error, SeamError::Unknown(_)),
        true,
        "a name found nowhere is Unknown",
    );
    // With no `PATH` at all the child has nowhere to look, and the runner's own is not one.
    rig.install(
        &mut runner,
        &format!("{}/decoy", rig.own_path_dir()),
        prints("decoy"),
    );
    let error = must_err(
        run_plain(rig, &mut runner, &command(&["decoy"], &[])),
        "run of a bare name by a child that has no PATH",
    );
    same(
        matches!(error, SeamError::Unknown(_)),
        true,
        "a bare name with no PATH in the environment is Unknown",
    );
}

fn argv_and_environment_are_exactly_what_was_given<R: ProcessRig>(rig: &R) {
    let mut runner = rig.runner();
    let program = format!("{}/bin/report", rig.scratch());
    rig.install(&mut runner, &program, Behavior::Reports);
    // What an earlier run of the same runner was given is not what this child sees.
    let earlier = command(&[&program], &[("SECRET", "1"), ("PATH", "/earlier")]);
    must(run_plain(rig, &mut runner, &earlier), "an earlier run");
    // A run that could not be started and one that was stopped leave nothing behind either.
    let clean = |runner: &mut R::Runner, after: &str| {
        let bare = command(&[&program], &[("PATH", "/nowhere")]);
        let result = must(run_plain(rig, runner, &bare), after);
        same(
            text(&result.stdout),
            format!("cwd {}\nenv PATH=/nowhere\n", rig.scratch()),
            &format!("what the child saw after {after}"),
        );
    };
    clean(&mut runner, "an earlier run");
    let unstartable = command(&["never-installed"], &[("SECRET_UNSTARTED", "1")]);
    must_err(
        run_plain(rig, &mut runner, &unstartable),
        "an earlier run that cannot be started",
    );
    clean(&mut runner, "a run that could not be started");
    let chatter = format!("{}/bin/chatter", rig.scratch());
    rig.install(&mut runner, &chatter, Behavior::Chatters { chunks: 4 });
    let stopped = command(&[&chatter], &[("SECRET_STOPPED", "1")]);
    let _ = runner.run(&ProcessSpec {
        command: &stopped,
        cwd: &rig.scratch(),
        heartbeat: &Beats::stopping_at(1),
        stdout_limit: 1 << 20,
    });
    clean(&mut runner, "a run that was stopped");
    let given = command(
        &[
            &program,
            "plain",
            "with space",
            "",
            "--flag=value",
            "\u{e9}",
        ],
        &[("A", "1"), ("B", ""), ("EQ", "a=b=c"), ("PATH", "/nowhere")],
    );
    let result = must(run_plain(rig, &mut runner, &given), "run");
    same(result.exit, Exit::Code(0), "the exit of the report");
    same(
        text(&result.stdout),
        format!(
            "cwd {}\narg plain\narg with space\narg \narg --flag=value\narg \u{e9}\n\
             env A=1\nenv B=\nenv EQ=a=b=c\nenv PATH=/nowhere\n",
            rig.scratch()
        ),
        "the working directory, the arguments and the whole environment the child saw",
    );
    // A later run is in the directory it is given, which is not the one of the first.
    let elsewhere = format!("{}/elsewhere", rig.scratch());
    rig.install(&mut runner, &format!("{elsewhere}/marker"), prints(""));
    let again = command(&[&program, "x"], &[("PATH", "/nowhere")]);
    let result = must(
        run_with(&mut runner, &again, &elsewhere, 1 << 20),
        "a run in another directory",
    );
    same(
        text(&result.stdout),
        format!("cwd {elsewhere}\narg x\nenv PATH=/nowhere\n"),
        "the working directory of a later run, and the environment of that run alone",
    );
}

fn no_shell_interprets_an_argument<R: ProcessRig>(rig: &R) {
    let mut runner = rig.runner();
    let program = format!("{}/bin/report", rig.scratch());
    rig.install(&mut runner, &program, Behavior::Reports);
    let args = [
        "$HOME",
        "*",
        "a;b",
        "`id`",
        "'q' \"r\"",
        "~",
        "$(x)",
        "a b|c",
        "&&",
        "> out",
    ];
    let mut argv = vec![program.as_str()];
    argv.extend(args);
    let result = must(
        run_plain(rig, &mut runner, &command(&argv, &[("PATH", "/nowhere")])),
        "run",
    );
    let seen: Vec<String> = text(&result.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("arg ").map(str::to_owned))
        .collect();
    same(
        seen,
        args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>(),
        "the arguments the child saw",
    );
}

fn exit_code_and_signal_are_distinguished<R: ProcessRig>(rig: &R) {
    let mut runner = rig.runner();
    let dir = format!("{}/bin", rig.scratch());
    rig.install(&mut runner, &format!("{dir}/ok"), prints(""));
    rig.install(
        &mut runner,
        &format!("{dir}/three"),
        Behavior::Prints {
            stdout: Vec::new(),
            stderr: Vec::new(),
            code: 3,
        },
    );
    rig.install(&mut runner, &format!("{dir}/killed"), Behavior::Killed(9));
    // Another signal than 9, and exit codes that are above 128 without being a signal.
    rig.install(
        &mut runner,
        &format!("{dir}/terminated"),
        Behavior::Killed(15),
    );
    rig.install(&mut runner, &format!("{dir}/hangup"), Behavior::Killed(1));
    rig.install(&mut runner, &format!("{dir}/high"), exits(200));
    rig.install(&mut runner, &format!("{dir}/as_killed"), exits(137));
    for (name, want) in [
        ("ok", Exit::Code(0)),
        ("three", Exit::Code(3)),
        ("killed", Exit::Signal(9)),
        ("terminated", Exit::Signal(15)),
        ("hangup", Exit::Signal(1)),
        ("high", Exit::Code(200)),
        ("as_killed", Exit::Code(137)),
    ] {
        let given = command(&[&format!("{dir}/{name}")], &[]);
        let result = must(run_plain(rig, &mut runner, &given), name);
        same(result.exit, want, &format!("how {name} ended"));
    }
}

fn unstartable_is_not_a_nonzero_exit<R: ProcessRig>(rig: &R) {
    let mut runner = rig.runner();
    let given = command(&[&format!("{}/bin/never-installed", rig.scratch())], &[]);
    let error = must_err(
        run_plain(rig, &mut runner, &given),
        "run of a program that is not there",
    );
    same(
        matches!(error, SeamError::Unknown(_)),
        true,
        "a program that cannot be started is Unknown, not an exit",
    );
}

fn output_is_bounded_and_its_tail_is_kept<R: ProcessRig>(rig: &R) {
    let mut runner = rig.runner();
    let program = format!("{}/bin/noisy", rig.scratch());
    let stdout: Vec<u8> = (0..4000_u32).map(|i| (i % 251) as u8).collect();
    let stderr: Vec<u8> = (0..3000_u32).map(|i| (i % 241) as u8).collect();
    rig.install(
        &mut runner,
        &program,
        Behavior::Prints {
            stdout: stdout.clone(),
            stderr: stderr.clone(),
            code: 0,
        },
    );
    let given = command(&[&program], &[]);
    // The limit of one run is the limit of that run: a first run that keeps everything does not
    // set the limit of the next, and a limit of 0 keeps nothing.
    let whole = must(
        run_with(&mut runner, &given, &rig.scratch(), 1 << 20),
        "a run that keeps everything",
    );
    same(
        whole.stdout,
        stdout.clone(),
        "the output kept under a big limit",
    );
    let result = must(run_with(&mut runner, &given, &rig.scratch(), 100), "run");
    same(
        result.stdout,
        stdout[stdout.len() - 100..].to_vec(),
        "the kept standard output",
    );
    same(
        result.stderr_tail.is_empty(),
        false,
        "the tail of the error output is kept",
    );
    same(
        stderr.ends_with(&result.stderr_tail),
        true,
        "the error output kept is its tail",
    );
    let none = must(
        run_with(&mut runner, &given, &rig.scratch(), 0),
        "a run with a limit of 0",
    );
    same(
        none.stdout,
        Vec::new(),
        "the output kept under a limit of 0",
    );
    // The error tail is that of the run: a run that wrote none has none.
    let quiet = format!("{}/bin/quiet", rig.scratch());
    rig.install(&mut runner, &quiet, prints("quiet"));
    let result = must(
        run_with(&mut runner, &command(&[&quiet], &[]), &rig.scratch(), 100),
        "a quiet run after a noisy one",
    );
    same(
        result.stderr_tail,
        Vec::new(),
        "the error tail of a run that wrote none",
    );
}

fn the_runner_beats_on_progress_and_stops_the_child_when_told<R: ProcessRig>(rig: &R) {
    let mut runner = rig.runner();
    let program = format!("{}/bin/chatter", rig.scratch());
    rig.install(&mut runner, &program, Behavior::Chatters { chunks: 4 });
    let given = command(&[&program], &[]);
    let cwd = rig.scratch();
    let spec = |heartbeat| ProcessSpec {
        command: &given,
        cwd: &cwd,
        heartbeat,
        stdout_limit: 1 << 20,
    };
    let counting = Beats::counting();
    let result = must(runner.run(&spec(&counting)), "run of a chatty child");
    same(
        result.exit,
        Exit::Code(0),
        "the chatty child ends by itself",
    );
    same(counting.count() >= 1, true, "the runner beats on progress");
    let stopping = Beats::stopping_at(2);
    let stopped = must_err(
        runner.run(&spec(&stopping)),
        "run of a child that is stopped",
    );
    same(
        stopped,
        SeamError::Refused(Reason::LeaseLost),
        "the answer to a Stop",
    );
    same(stopping.count() >= 2, true, "the beats before the Stop");
    // A Stop ends one run: the runner runs the next child as it did the first.
    let after = must(runner.run(&spec(&Beats::counting())), "a run after a Stop");
    same(
        after.exit,
        Exit::Code(0),
        "the chatty child ends by itself after a run was stopped",
    );
}

fn a_stopped_child_and_its_process_group_are_gone<R: ProcessRig>(rig: &R) {
    let mut runner = rig.runner();
    let program = format!("{}/bin/holder", rig.scratch());
    rig.install(
        &mut runner,
        &program,
        Behavior::Holds {
            name: "held".to_owned(),
        },
    );
    let running = Cell::new(false);
    let watcher = |_: u32| running.set(rig.alive("held") & rig.alive("held.group"));
    let beats = Beats::stopping_at(1).watched_by(&watcher);
    let given = command(&[&program], &[]);
    let _ = runner.run(&ProcessSpec {
        command: &given,
        cwd: &rig.scratch(),
        heartbeat: &beats,
        stdout_limit: 1 << 20,
    });
    same(
        running.get(),
        true,
        "the child and its group ran when the Stop came",
    );
    same(rig.alive("held"), false, "the child after the Stop");
    same(
        rig.alive("held.group"),
        false,
        "the process group of the child after the Stop",
    );
}
