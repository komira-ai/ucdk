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
    /// `env <key>=<value>` for each variable of its environment, sorted by key (by bytes), then
    /// ends with code 0. Each text is written byte for byte as the child has it, so an argument
    /// that holds a newline is written over more than one line.
    Reports,
    /// Writes `chunks` chunks of output one after the other, each written and flushed on its
    /// own, then ends with code 0.
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
    // Every directory of the `PATH` is searched, however many there are, and a directory whose
    // name holds a space is one directory.
    let long: Vec<String> = (1..=5)
        .map(|i| format!("{}/none-{i}", rig.scratch()))
        .chain([child_dir.clone()])
        .collect();
    let result = must(
        run_plain(
            rig,
            &mut runner,
            &command(&["tool"], &[("PATH", &long.join(":"))]),
        ),
        "run of a bare name found in the sixth directory of the PATH",
    );
    same(
        text(&result.stdout),
        "child".to_owned(),
        "the program that ran with a name found in the sixth directory of the PATH",
    );
    let spaced = format!("{}/with space", rig.scratch());
    rig.install(&mut runner, &format!("{spaced}/tool"), prints("spaced"));
    let result = must(
        run_plain(rig, &mut runner, &command(&["tool"], &[("PATH", &spaced)])),
        "run of a bare name found in a directory whose name holds a space",
    );
    same(
        text(&result.stdout),
        "spaced".to_owned(),
        "the program that ran from a directory whose name holds a space",
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
    // A directory of the `PATH` that is not absolute is relative to the working directory of the
    // run, whichever directory that is.
    let there = format!("{}/elsewhere", rig.scratch());
    rig.install(
        &mut runner,
        &format!("{there}/child-bin/tool"),
        prints("there"),
    );
    for (cwd, want) in [(rig.scratch(), "child"), (there, "there")] {
        // The argument tells the two runs apart for a rig that scripts by command.
        let relative = command(&["tool", want], &[("PATH", "child-bin")]);
        let result = must(
            run_with(&mut runner, &relative, &cwd, 1 << 20),
            "run of a bare name found by a directory of the PATH that is relative",
        );
        same(
            text(&result.stdout),
            want.to_owned(),
            &format!("the program that ran with a relative directory of the PATH in {cwd}"),
        );
    }
    // Nothing of an earlier run is remembered: a name that the `PATH` of an earlier run had is
    // Unknown when this run has none, and only a variable called `PATH` is the `PATH`. The name
    // is on no `PATH` that the runner could have of its own.
    rig.install(&mut runner, &format!("{child_dir}/kept"), prints("kept"));
    let result = must(
        run_plain(
            rig,
            &mut runner,
            &command(&["kept"], &[("PATH", &child_dir)]),
        ),
        "run of a bare name found on the PATH given",
    );
    same(
        text(&result.stdout),
        "kept".to_owned(),
        "the program that ran on the PATH given",
    );
    must_err(
        run_plain(rig, &mut runner, &command(&["kept"], &[])),
        "run of a bare name by a child that has no PATH, after one that had",
    );
    for key in ["path", "Path"] {
        must_err(
            run_plain(rig, &mut runner, &command(&["kept"], &[(key, &child_dir)])),
            "run of a bare name by a child whose variable is not called PATH",
        );
    }
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
    // The working directory of the run is not on the `PATH` either: a bare name is looked for
    // on the `PATH` and nowhere else.
    rig.install(
        &mut runner,
        &format!("{}/lurker", rig.scratch()),
        prints("lurker"),
    );
    let error = must_err(
        run_plain(
            rig,
            &mut runner,
            &command(&["lurker"], &[("PATH", &child_dir)]),
        ),
        "run of a bare name that is in the working directory and not on the PATH",
    );
    same(
        matches!(error, SeamError::Unknown(_)),
        true,
        "a bare name found only in the working directory is Unknown",
    );
    // A name with a `/` in it is not a bare name: it is relative to the working directory of the
    // run and is not looked for on the `PATH`, nor relative to the runner's own directory.
    let elsewhere = format!("{}/elsewhere", rig.scratch());
    rig.install(&mut runner, &format!("{elsewhere}/marker"), prints(""));
    let relative = command(&["child-bin/tool"], &[("PATH", &rig.scratch())]);
    rig.install(
        &mut runner,
        &format!("{child_dir}/tool"),
        prints("relative"),
    );
    let result = must(
        run_with(&mut runner, &relative, &rig.scratch(), 1 << 20),
        "run of a name with a slash in the directory it is relative to",
    );
    same(
        text(&result.stdout),
        "relative".to_owned(),
        "the program found relative to the working directory",
    );
    let error = must_err(
        run_with(&mut runner, &relative, &elsewhere, 1 << 20),
        "run of a name with a slash in a directory it is not relative to",
    );
    same(
        matches!(error, SeamError::Unknown(_)),
        true,
        "a name with a slash is not looked for on the PATH",
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
    let args = [
        "plain",
        "with space",
        "",
        "--flag=value",
        "\u{e9}",
        " lead",
        "trail ",
        "\ttab",
        "back\\slash",
        "a\\ b",
        "l1\nl2",
        "UPPER and lower",
    ];
    let mut argv = vec![program.as_str()];
    argv.extend(args);
    let given = command(
        &argv,
        &[
            ("A", "1"),
            ("B", ""),
            ("BS", "a\\b"),
            ("EQ", "a=b=c"),
            ("NA", "\u{e9}"),
            ("NL", "l1\nl2"),
            ("PATH", "/nowhere"),
            ("SP", " x "),
            ("lower", "MiXed"),
        ],
    );
    let result = must(run_plain(rig, &mut runner, &given), "run");
    same(result.exit, Exit::Code(0), "the exit of the report");
    let want_args: String = args.iter().map(|a| format!("arg {a}\n")).collect();
    same(
        text(&result.stdout),
        format!(
            "cwd {}\n{want_args}env A=1\nenv B=\nenv BS=a\\b\nenv EQ=a=b=c\nenv NA=\u{e9}\n\
             env NL=l1\nl2\nenv PATH=/nowhere\nenv SP= x \nenv lower=MiXed\n",
            rig.scratch()
        ),
        "the working directory, the arguments and the whole environment the child saw",
    );
    // A program whose path holds a space is one program.
    let spaced = format!("{}/with space/report", rig.scratch());
    rig.install(&mut runner, &spaced, Behavior::Reports);
    let result = must(
        run_plain(rig, &mut runner, &command(&[&spaced, "x"], &[])),
        "run of a program whose path holds a space",
    );
    same(
        text(&result.stdout),
        format!("cwd {}\narg x\n", rig.scratch()),
        "what a program whose path holds a space reported",
    );
    // No environment given is no environment, and no `PATH` given is no `PATH`.
    let small: [(&[(&str, &str)], &str); 2] = [(&[], ""), (&[("A", "1")], "env A=1\n")];
    for (env, want) in small {
        let result = must(
            run_plain(rig, &mut runner, &command(&[&program], env)),
            "a run with a small environment",
        );
        same(
            text(&result.stdout),
            format!("cwd {}\n{want}", rig.scratch()),
            "what the child saw of an environment of its own size",
        );
    }
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
    // The codes a shell gives its own meaning to are codes, and so are signals that are neither
    // 1, 9 nor 15.
    for (name, code) in [
        ("one", 1),
        ("cannot_run", 126),
        ("not_found", 127),
        ("max", 255),
    ] {
        rig.install(&mut runner, &format!("{dir}/{name}"), exits(code));
    }
    for (name, signal) in [("interrupted", 2), ("aborted", 6), ("segfault", 11)] {
        rig.install(
            &mut runner,
            &format!("{dir}/{name}"),
            Behavior::Killed(signal),
        );
    }
    for (name, want) in [
        ("ok", Exit::Code(0)),
        ("three", Exit::Code(3)),
        ("killed", Exit::Signal(9)),
        ("terminated", Exit::Signal(15)),
        ("hangup", Exit::Signal(1)),
        ("high", Exit::Code(200)),
        ("as_killed", Exit::Code(137)),
        ("one", Exit::Code(1)),
        ("cannot_run", Exit::Code(126)),
        ("not_found", Exit::Code(127)),
        ("max", Exit::Code(255)),
        ("interrupted", Exit::Signal(2)),
        ("aborted", Exit::Signal(6)),
        ("segfault", Exit::Signal(11)),
    ] {
        let given = command(&[&format!("{dir}/{name}")], &[]);
        let result = must(run_plain(rig, &mut runner, &given), name);
        same(result.exit, want, &format!("how {name} ended"));
    }
    // A child that fails has its output kept all the same.
    rig.install(
        &mut runner,
        &format!("{dir}/fails"),
        Behavior::Prints {
            stdout: b"out".to_vec(),
            stderr: b"err".to_vec(),
            code: 1,
        },
    );
    let result = must(
        run_plain(rig, &mut runner, &command(&[&format!("{dir}/fails")], &[])),
        "run of a child that fails after it wrote",
    );
    same(result.exit, Exit::Code(1), "how the failing child ended");
    same(
        result.stdout,
        b"out".to_vec(),
        "the output of a failing child",
    );
    same(
        result.stderr_tail,
        b"err".to_vec(),
        "the error output of a failing child",
    );
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
    // Nor is it the tail of a run that failed before it.
    let failing = format!("{}/bin/failing", rig.scratch());
    rig.install(
        &mut runner,
        &failing,
        Behavior::Prints {
            stdout: b"x".to_vec(),
            stderr: b"boom".to_vec(),
            code: 1,
        },
    );
    must(
        run_with(&mut runner, &command(&[&failing], &[]), &rig.scratch(), 100),
        "a failing run",
    );
    let result = must(
        run_with(&mut runner, &command(&[&quiet], &[]), &rig.scratch(), 100),
        "a quiet run after a failing one",
    );
    same(
        result.stderr_tail,
        Vec::new(),
        "the error tail of a run that wrote none after a run that failed",
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
    // A child that keeps writing keeps the runner beating: one beat at least for each chunk.
    let long = format!("{}/bin/long-chatter", rig.scratch());
    rig.install(&mut runner, &long, Behavior::Chatters { chunks: 50 });
    let long_command = command(&[&long], &[]);
    let beats = Beats::counting();
    must(
        runner.run(&ProcessSpec {
            command: &long_command,
            cwd: &cwd,
            heartbeat: &beats,
            stdout_limit: 1 << 20,
        }),
        "run of a child that writes many chunks",
    );
    same(
        beats.count() >= 50,
        true,
        "the runner beats for each chunk a long chatty child writes",
    );
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
