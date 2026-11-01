// SPDX-License-Identifier: Apache-2.0
//! What `Buck2` and `Bazel` share: running one invocation, finding what ended up on the disk,
//! reading a test report and copying an output out.

use ucdk_release::invocation::{self, Command, Invocation, Verb};
use ucdk_release::test_report;
use ucdk_release_proto::v1::{BuildSystemKind, Reason};
use ucdk_release_seams::{
    EntryWriter, Exit, FileKind, Heartbeat, Located, OutputFiles, ProcessResult, ProcessRunner,
    ProcessSpec, SeamError, Shape, TestOutcome, entry_path,
};

/// The most standard output one invocation keeps: a listing is a few lines, and the rest is
/// there for the tail of a failure.
const STDOUT_KEPT: usize = 1 << 20;
/// The bytes of each stream that a failure quotes.
const TAIL: usize = 4096;

/// A process runner and a reader of files, the directory the build system runs in, and the
/// steps of a build and a test that do not depend on the kind.
pub(crate) struct Shared<R, F> {
    runner: R,
    files: F,
    root: String,
}

fn refused() -> SeamError {
    SeamError::Refused(Reason::BuildOutput)
}

/// The command of `inv` for `verb`: the layering of the design, with no effect.
pub(crate) fn layered(inv: &Invocation<'_>, verb: Verb) -> Result<Command, SeamError> {
    let report = if verb == Verb::Test { inv.report } else { None };
    invocation::command(&Invocation {
        verb,
        report,
        ..*inv
    })
    .map_err(|error| SeamError::Permanent(error.to_string()))
}

/// The last bytes of `bytes`, as text.
fn tail_of(bytes: &[u8]) -> String {
    let from = bytes.len().saturating_sub(TAIL);
    String::from_utf8_lossy(&bytes[from..])
        .trim_end()
        .to_owned()
}

/// How the invocation ended and what it printed last.
fn account(what: &str, result: &ProcessResult) -> String {
    let ended = match result.exit {
        Exit::Code(code) => format!("exited {code}"),
        Exit::Signal(signal) => format!("was killed by signal {signal}"),
    };
    format!(
        "{what} {ended}; it printed last on standard error: {}; and on standard output: {}",
        tail_of(&result.stderr_tail),
        tail_of(&result.stdout)
    )
}

impl<R: ProcessRunner, F: OutputFiles> Shared<R, F> {
    pub(crate) fn new(runner: R, files: F, root: String) -> Self {
        Self {
            runner,
            files,
            root,
        }
    }

    /// Runs `command` in the root. The result is what it ended with, whatever that was.
    fn start(
        &mut self,
        command: &Command,
        heartbeat: &dyn Heartbeat,
    ) -> Result<ProcessResult, SeamError> {
        self.runner.run(&ProcessSpec {
            command,
            cwd: &self.root,
            heartbeat,
            stdout_limit: STDOUT_KEPT,
        })
    }

    /// Runs the `verb` of `inv` and answers what it printed on standard output; a nonzero exit
    /// or a signal is `Permanent` and carries the tail of the output.
    pub(crate) fn succeed(
        &mut self,
        inv: &Invocation<'_>,
        verb: Verb,
        heartbeat: &dyn Heartbeat,
    ) -> Result<Vec<u8>, SeamError> {
        let command = layered(inv, verb)?;
        let result = self.start(&command, heartbeat)?;
        if result.exit == Exit::Code(0) {
            Ok(result.stdout)
        } else {
            Err(SeamError::Permanent(account(
                &format!("the {verb} of {}", inv.target.label),
                &result,
            )))
        }
    }

    /// Checks the output a listing named with `OutputFiles`, never with the listing: nothing
    /// there and a symlink are `BUILD_OUTPUT`, and so is a symlink anywhere below a directory.
    pub(crate) fn locate(&mut self, label: &str, path: &str) -> Result<Vec<Located>, SeamError> {
        let shape = match self.files.stat(path)? {
            Some(FileKind::File) => Shape::File,
            Some(FileKind::Directory) => {
                let below = self.files.list(path)?;
                if below.iter().any(|entry| entry.kind == FileKind::Symlink) {
                    return Err(refused());
                }
                Shape::Directory
            }
            Some(FileKind::Symlink) | None => return Err(refused()),
        };
        Ok(vec![Located {
            label: label.to_owned(),
            path: path.to_owned(),
            shape,
        }])
    }

    /// Runs the test of `inv` and reads its report: it passed when it exited 0 and the report
    /// shows a test that ran.
    pub(crate) fn test(
        &mut self,
        kind: BuildSystemKind,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<TestOutcome, SeamError> {
        let command = layered(inv, Verb::Test)?;
        let result = self.start(&command, heartbeat)?;
        // `layered` has refused a test with no report path.
        let report = inv.report.unwrap_or_default();
        let bytes = match self.files.stat(report)? {
            Some(FileKind::File) => Some(self.files.read(report)?),
            _ => None,
        };
        let ran = test_report::ran(kind, bytes.as_deref());
        Ok(TestOutcome {
            passed: (result.exit == Exit::Code(0)) & (ran > 0),
            ran,
            tail: account(&format!("the test of {}", inv.target.label), &result),
        })
    }

    /// Writes the files of `out` into `into`: a file as its own name, and a directory as the
    /// files below it, each under the directory's name. What is found now must be what was
    /// located: a path that changed shape, or that holds a symlink, is `BUILD_OUTPUT`. A
    /// directory with nothing in it has no file to write, and so leaves nothing.
    pub(crate) fn copy_out(
        &mut self,
        out: &Located,
        into: &mut dyn EntryWriter,
    ) -> Result<(), SeamError> {
        let top = out.path.trim_end_matches('/');
        let name = top.rsplit('/').next().unwrap_or_default();
        match (out.shape, self.files.stat(top)?) {
            (Shape::File, Some(FileKind::File)) => into.put_file(name, &self.files.read(top)?),
            (Shape::Directory, Some(FileKind::Directory)) => {
                for entry in self.files.list(top)? {
                    match entry.kind {
                        FileKind::File => {
                            let bytes = self.files.read(&entry_path(top, &entry.rel)?)?;
                            into.put_file(&format!("{name}/{}", entry.rel), &bytes)?;
                        }
                        FileKind::Directory => {}
                        FileKind::Symlink => return Err(refused()),
                    }
                }
                Ok(())
            }
            _ => Err(refused()),
        }
    }
}
