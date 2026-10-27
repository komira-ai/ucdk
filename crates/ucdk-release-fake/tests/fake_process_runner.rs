// SPDX-License-Identifier: Apache-2.0
//! The fake process runner: scripted by argument vector and environment, loud about the rest.

use std::time::Duration;

use ucdk_core::{Clock, ManualClock};
use ucdk_release_fake::{FakeHeartbeat, FakeProcessRunner};
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Command, Exit, ProcessResult, ProcessRunner, ProcessSpec, SeamError};

fn command(program: &str, path: Option<&str>) -> Command {
    Command {
        argv: vec![program.to_owned(), "build".to_owned()],
        env: path
            .map(|p| vec![("PATH".to_owned(), p.to_owned())])
            .unwrap_or_default(),
    }
}

fn run(
    runner: &mut FakeProcessRunner,
    command: &Command,
    heartbeat: &FakeHeartbeat,
    limit: usize,
) -> Result<ProcessResult, SeamError> {
    runner.run(&ProcessSpec {
        command,
        cwd: "/work",
        heartbeat,
        stdout_limit: limit,
    })
}

fn setup() -> (ManualClock, FakeProcessRunner, FakeHeartbeat) {
    let clock = ManualClock::at(1_000);
    let runner = FakeProcessRunner::new(&clock);
    let heartbeat = FakeHeartbeat::new(&clock);
    (clock, runner, heartbeat)
}

#[test]
fn a_scripted_call_answers_and_is_recorded() {
    let (_, mut runner, hb) = setup();
    let cmd = command("/opt/bin/buck2", None);
    runner.program("/opt/bin/buck2");
    runner.script(cmd.clone(), Exit::Code(0), b"out", b"err");
    let result = run(&mut runner, &cmd, &hb, 100).unwrap();
    assert_eq!(
        result,
        ProcessResult {
            exit: Exit::Code(0),
            stdout: b"out".to_vec(),
            stderr_tail: b"err".to_vec()
        }
    );
    assert_eq!(runner.calls().len(), 1);
    assert_eq!(runner.calls()[0].command, cmd);
    assert_eq!(runner.calls()[0].cwd, "/work");
}

#[test]
#[should_panic(expected = "a call nobody scripted")]
fn an_unscripted_call_fails_the_test() {
    let (_, mut runner, hb) = setup();
    runner.program("/opt/bin/buck2");
    run(&mut runner, &command("/opt/bin/buck2", None), &hb, 100).unwrap();
}

#[test]
#[should_panic(expected = "a call nobody scripted")]
fn an_unscripted_call_fails_the_test_even_when_its_program_was_never_registered() {
    let (_, mut runner, hb) = setup();
    run(&mut runner, &command("/never/registered", None), &hb, 100).unwrap();
}

#[test]
fn a_script_matches_the_whole_argument_vector_and_environment() {
    let (_, mut runner, hb) = setup();
    runner.program("/opt/bin/buck2");
    runner.script(command("/opt/bin/buck2", None), Exit::Code(0), b"", b"");
    let mut other = command("/opt/bin/buck2", None);
    other.env.push(("K".to_owned(), "v".to_owned()));
    runner.script(other.clone(), Exit::Code(3), b"", b"");
    assert_eq!(
        run(&mut runner, &other, &hb, 1).unwrap().exit,
        Exit::Code(3)
    );
}

#[test]
fn a_bare_name_is_resolved_on_the_scripted_childs_path() {
    let (_, mut runner, hb) = setup();
    runner.program("/child/bin/buck2");
    let on_childs_path = command("buck2", Some("/usr/bin:/child/bin"));
    runner.script(on_childs_path.clone(), Exit::Code(0), b"", b"");
    assert!(run(&mut runner, &on_childs_path, &hb, 1).is_ok());
    // The same name with a PATH that does not hold it, or with none at all, is not startable,
    // wherever the runner's own search would have found it.
    for cmd in [command("buck2", Some("/usr/bin")), command("buck2", None)] {
        runner.script(cmd.clone(), Exit::Code(0), b"", b"");
        assert!(matches!(
            run(&mut runner, &cmd, &hb, 1),
            Err(SeamError::Unknown(_))
        ));
    }
    // An absolute name nobody installed is unstartable too, and so is an empty vector.
    let nowhere = command("/nowhere/buck2", None);
    runner.script(nowhere.clone(), Exit::Code(0), b"", b"");
    assert!(run(&mut runner, &nowhere, &hb, 1).is_err());
    let empty = Command {
        argv: Vec::new(),
        env: Vec::new(),
    };
    runner.script(empty.clone(), Exit::Code(0), b"", b"");
    assert!(run(&mut runner, &empty, &hb, 1).is_err());
}

#[test]
fn faults_kill_signal_or_refuse_to_start() {
    let (_, mut runner, hb) = setup();
    let cmd = command("/opt/bin/buck2", None);
    runner.program("/opt/bin/buck2");
    runner.script(cmd.clone(), Exit::Code(0), b"", b"");
    runner.die_at_call(1);
    runner.signal(2);
    assert!(matches!(
        run(&mut runner, &cmd, &hb, 1),
        Err(SeamError::Unknown(_))
    ));
    let signalled = run(&mut runner, &cmd, &hb, 1).unwrap();
    assert_eq!(signalled.exit, Exit::Signal(9));
    assert_eq!(run(&mut runner, &cmd, &hb, 1).unwrap().exit, Exit::Code(0));
    runner.unstartable();
    assert!(matches!(
        run(&mut runner, &cmd, &hb, 1),
        Err(SeamError::Unknown(_))
    ));
}

#[test]
fn output_over_the_limit_keeps_its_tail() {
    let (_, mut runner, hb) = setup();
    let cmd = command("/opt/bin/buck2", None);
    runner.program("/opt/bin/buck2");
    runner.script(cmd.clone(), Exit::Code(0), b"0123456789", b"");
    assert_eq!(run(&mut runner, &cmd, &hb, 4).unwrap().stdout, b"6789");
}

#[test]
fn beats_advance_the_clock_before_each_beat_and_stop_ends_the_call() {
    let (clock, mut runner, hb) = setup();
    let cmd = command("/opt/bin/buck2", None);
    runner.program("/opt/bin/buck2");
    runner.script(cmd.clone(), Exit::Code(0), b"", b"");
    runner.beats(3, Duration::from_secs(2));
    run(&mut runner, &cmd, &hb, 1).unwrap();
    assert_eq!(hb.readings(), [3_000, 5_000, 7_000]);
    let stopping = FakeHeartbeat::new(&clock).stop_at(2);
    assert_eq!(
        run(&mut runner, &cmd, &stopping, 1),
        Err(SeamError::Refused(Reason::LeaseLost))
    );
    assert_eq!(stopping.beats(), 2);
}

#[test]
fn silent_for_advances_the_clock_and_never_beats() {
    let (clock, mut runner, hb) = setup();
    let cmd = command("/opt/bin/buck2", None);
    runner.program("/opt/bin/buck2");
    runner.script(cmd.clone(), Exit::Code(0), b"", b"");
    runner.silent_for(Duration::from_secs(1_000));
    run(&mut runner, &cmd, &hb, 1).unwrap();
    assert_eq!(hb.beats(), 0);
    assert_eq!(clock.now_millis(), 1_000 + 1_000_000);
}
