// SPDX-License-Identifier: Apache-2.0
//! A rig for the build-system kit over a scripted process runner and the fake files: the runner
//! answers each invocation the way the build tool of its kind would in the world of the
//! scenario, writes the output of a build into the tree, and writes the report of a test.

use std::sync::{Arc, Mutex};

use ucdk_release::invocation::{self, Command, Invocation, Verb};
use ucdk_release_build::{Bazel, Buck2};
use ucdk_release_fake::FakeOutputFiles;
use ucdk_release_proto::v1::{BuildSystemKind, Reason};
use ucdk_release_seams::kit::{BuildSystemRig, BuildWorld};
use ucdk_release_seams::{
    BuildSystem, EntryWriter, Exit, Heartbeat, Located, ProcessResult, ProcessRunner, ProcessSpec,
    SeamError, TestOutcome,
};

use super::{Handle, ROOT};

/// Where Buck2 puts the output: a path that holds a space.
const BUCK2_OUT: &str = "/build out/pkg/out";
/// Where Bazel's execution root is, and the output below it as `cquery` prints it.
const EXEC_ROOT: &str = "/build out/execroot";
const BAZEL_REL: &str = "bazel-out/bin/pkg/out";

type Log = Arc<Mutex<Vec<Command>>>;

/// What an invocation of the tool writes: the same for the same command, and different for
/// another, as bytes built from a command are.
fn render(command: &Command, part: &str) -> Vec<u8> {
    format!("built by the scripted tool\n{command:?}\n{part}\n").into_bytes()
}

fn finished(code: i32, stdout: &str, stderr: &str) -> Result<ProcessResult, SeamError> {
    Ok(ProcessResult {
        exit: Exit::Code(code),
        stdout: stdout.as_bytes().to_vec(),
        stderr_tail: stderr.as_bytes().to_vec(),
    })
}

/// The tool, in one world.
struct Scripted {
    kind: BuildSystemKind,
    world: BuildWorld,
    files: Handle<FakeOutputFiles>,
    log: Log,
}

impl Scripted {
    fn out_path(&self) -> String {
        out_path(self.kind)
    }

    /// What a build writes into the tree.
    fn lay_out(&self, command: &Command) {
        let out = self.out_path();
        self.files.with(|files| match &self.world {
            BuildWorld::DirectoryOutput => {
                files.file(&format!("{out}/a"), &render(command, "a"));
                files.file(&format!("{out}/sub/b"), &render(command, "b"));
            }
            BuildWorld::SymlinkInOutput => files.symlink(&out),
            BuildWorld::SymlinkInDirectory => {
                files.file(&format!("{out}/a"), &render(command, "a"));
                files.symlink(&format!("{out}/sub/link"));
            }
            BuildWorld::NoOutput => {}
            BuildWorld::TwoOutputs => {
                files.file(&out, &render(command, ""));
                files.file(&format!("{out}2"), &render(command, ""));
            }
            _ => files.file(&out, &render(command, "")),
        });
    }

    fn build(&self, command: &Command) -> Result<ProcessResult, SeamError> {
        if let BuildWorld::BuildFails { head, tail } = &self.world {
            return finished(1, "", &format!("{head}{tail}"));
        }
        self.lay_out(command);
        let out = self.out_path();
        let listing = if self.kind == BuildSystemKind::Buck2 {
            match &self.world {
                BuildWorld::NoOutput => String::new(),
                BuildWorld::TwoOutputs => format!("root//pkg:out {out}\nroot//pkg:out {out}2\n"),
                BuildWorld::ListsAnotherTarget => format!("root//other:thing {out}\n"),
                BuildWorld::ListsAHostilePath { path } => format!("root//pkg:out {path}\n"),
                _ => format!("root//pkg:out {out}\n"),
            }
        } else {
            String::new()
        };
        finished(0, &listing, "")
    }

    /// Bazel's `cquery --output=files`: the output files relative to the execution root.
    fn query(&self) -> Result<ProcessResult, SeamError> {
        let listing = match &self.world {
            BuildWorld::NoOutput => String::new(),
            BuildWorld::TwoOutputs => format!("{BAZEL_REL}\n{BAZEL_REL}2\n"),
            BuildWorld::ListsAHostilePath { path } => format!("{path}\n"),
            _ => format!("{BAZEL_REL}\n"),
        };
        finished(0, &listing, "")
    }

    /// Bazel's `info execution_root`. A listing that names another target has no form in
    /// Bazel's, whose `cquery` prints files only; the stand-in is an execution root that is not
    /// a path.
    fn info(&self) -> Result<ProcessResult, SeamError> {
        let root = if self.world == BuildWorld::ListsAnotherTarget {
            "relative/root"
        } else {
            EXEC_ROOT
        };
        finished(0, &format!("{root}\n"), "")
    }

    fn test(&self, command: &Command) -> Result<ProcessResult, SeamError> {
        let buck2 = self.kind == BuildSystemKind::Buck2;
        let report = command
            .argv
            .iter()
            .find_map(|arg| {
                arg.strip_prefix("--event-log=")
                    .or_else(|| arg.strip_prefix("--build_event_json_file="))
            })
            .expect("a test names its report");
        let ran = if buck2 {
            "{\"TestResult\":{\"status\":\"PASS\"}}\n"
        } else {
            "{\"testSummary\":{\"totalRunCount\":1,\"overallStatus\":\"PASSED\"}}\n"
        };
        let nothing = if buck2 {
            "{\"Event\":{}}\n"
        } else {
            "{\"started\":{}}\n"
        };
        let skipped = if buck2 {
            "{\"TestResult\":{\"status\":\"SKIP\"}}\n"
        } else {
            "{\"testSummary\":{\"totalRunCount\":4,\"overallStatus\":\"FAILED_TO_BUILD\"}}\n"
        };
        let (written, code, stderr) = match &self.world {
            BuildWorld::TestRunsNothing => (Some(nothing), 0, String::new()),
            BuildWorld::TestSkipsEverything => (Some(skipped), 0, String::new()),
            BuildWorld::TestFails { tail } => (Some(ran), 1, tail.clone()),
            BuildWorld::ReportMissing => (None, 0, String::new()),
            BuildWorld::ReportUnparseable => (Some("this is not a report\n"), 0, String::new()),
            _ => (Some(ran), 0, String::new()),
        };
        if let Some(text) = written {
            self.files.with(|files| files.file(report, text.as_bytes()));
        }
        finished(code, "", &stderr)
    }
}

impl ProcessRunner for Scripted {
    fn run(&mut self, spec: &ProcessSpec<'_>) -> Result<ProcessResult, SeamError> {
        let command = spec.command;
        self.log
            .lock()
            .expect("the log is not held across a panic")
            .push(command.clone());
        if self.world == BuildWorld::Unstartable {
            return Err(SeamError::Unknown("the tool cannot be started".to_owned()));
        }
        if let BuildWorld::Progress { events } = self.world {
            for _ in 0..events {
                spec.heartbeat
                    .beat()
                    .map_err(|_| SeamError::Refused(Reason::LeaseLost))?;
            }
        }
        let verb = command
            .argv
            .iter()
            .skip(1)
            .map(String::as_str)
            .find(|word| matches!(*word, "build" | "test" | "cquery" | "info"))
            .expect("an invocation names its verb");
        match verb {
            "build" => self.build(command),
            "cquery" => self.query(),
            "info" => self.info(),
            _ => self.test(command),
        }
    }
}

fn out_path(kind: BuildSystemKind) -> String {
    if kind == BuildSystemKind::Buck2 {
        BUCK2_OUT.to_owned()
    } else {
        format!("{EXEC_ROOT}/{BAZEL_REL}")
    }
}

enum Inner {
    Buck2(Buck2<Scripted, Handle<FakeOutputFiles>>),
    Bazel(Bazel<Scripted, Handle<FakeOutputFiles>>),
}

/// The build system under test, with the commands its tool was given.
pub struct System {
    inner: Inner,
    log: Log,
}

impl BuildSystem for System {
    fn kind(&self) -> BuildSystemKind {
        match &self.inner {
            Inner::Buck2(system) => system.kind(),
            Inner::Bazel(system) => system.kind(),
        }
    }

    fn build(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<Vec<Located>, SeamError> {
        match &mut self.inner {
            Inner::Buck2(system) => system.build(inv, heartbeat),
            Inner::Bazel(system) => system.build(inv, heartbeat),
        }
    }

    fn test(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<TestOutcome, SeamError> {
        match &mut self.inner {
            Inner::Buck2(system) => system.test(inv, heartbeat),
            Inner::Bazel(system) => system.test(inv, heartbeat),
        }
    }

    fn copy_out(&mut self, out: &Located, into: &mut dyn EntryWriter) -> Result<(), SeamError> {
        match &mut self.inner {
            Inner::Buck2(system) => system.copy_out(out, into),
            Inner::Bazel(system) => system.copy_out(out, into),
        }
    }
}

/// The rig of one kind.
pub struct Rig {
    pub kind: BuildSystemKind,
}

impl BuildSystemRig for Rig {
    type System = System;

    fn system(&self, world: &BuildWorld) -> System {
        let files = Handle::new(FakeOutputFiles::new());
        let log: Log = Arc::default();
        let runner = Scripted {
            kind: self.kind,
            world: world.clone(),
            files: files.clone(),
            log: Arc::clone(&log),
        };
        let inner = if self.kind == BuildSystemKind::Buck2 {
            Inner::Buck2(Buck2::new(runner, files, ROOT))
        } else {
            Inner::Bazel(Bazel::new(runner, files, ROOT))
        };
        System { inner, log }
    }

    fn later(&self) {}

    fn output_of(&self, inv: &Invocation<'_>, world: &BuildWorld) -> Vec<(String, Vec<u8>)> {
        let command = invocation::command(&Invocation {
            verb: Verb::Build,
            report: None,
            ..*inv
        })
        .expect("the build layers");
        if *world == BuildWorld::DirectoryOutput {
            vec![
                ("out/a".to_owned(), render(&command, "a")),
                ("out/sub/b".to_owned(), render(&command, "b")),
            ]
        } else {
            vec![("out".to_owned(), render(&command, ""))]
        }
    }

    fn listed_path(&self, _world: &BuildWorld) -> String {
        out_path(self.kind)
    }

    fn executed(&self, system: &System) -> Vec<Command> {
        system
            .log
            .lock()
            .expect("the log is not held across a panic")
            .clone()
    }
}
