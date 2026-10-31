// SPDX-License-Identifier: Apache-2.0
//! What the tests of this crate share: a handle that lets a test look inside a fake after the
//! build system under test has taken it, the build system and the target of every case, an
//! entry writer that keeps what it is given, and a reader of files that fails on request.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ucdk_release::invocation::{self, Command, Invocation, Verb};
use ucdk_release_fake::{FakeOutputFiles, FakeProcessRunner};
use ucdk_release_proto::v1::{BuildStamp, BuildSystem as Authored, BuildSystemKind, Target};
use ucdk_release_seams::kit::Scenario;
use ucdk_release_seams::{
    DirEntry, EntryWriter, FileKind, Heartbeat, OutputFiles, ProcessResult, ProcessRunner,
    ProcessSpec, SeamError, Stop,
};

pub mod scripted;

/// The directory a build system runs in.
pub const ROOT: &str = "/work/repo";
/// The one target every case builds and tests.
pub const LABEL: &str = "//pkg:out";

/// A heartbeat that never stops anything.
pub struct Never;

impl Heartbeat for Never {
    fn beat(&self) -> Result<(), Stop> {
        Ok(())
    }
}

/// Bytes as the program of `tests/child` reads them: hexadecimal, and `-` for none.
pub fn hex(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        "-".to_owned()
    } else {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Runs the scenario called `name` of a kit on `rig`, and fails the test with its reason.
pub fn run_scenario<R>(scenarios: &[Scenario<R>], name: &str, rig: &R) {
    let Some(scenario) = scenarios.iter().find(|s| s.name == name) else {
        panic!("the kit has no scenario called {name}");
    };
    if let Err(why) = scenario.run_on(rig) {
        panic!("{name} failed: {why}");
    }
}

/// A value several owners can reach, so that a test keeps a way into a fake it has handed to a
/// build system.
pub struct Handle<T>(Arc<Mutex<T>>);

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> Handle<T> {
    pub fn new(inner: T) -> Self {
        Self(Arc::new(Mutex::new(inner)))
    }

    pub fn with<U>(&self, f: impl FnOnce(&mut T) -> U) -> U {
        f(&mut self
            .0
            .lock()
            .expect("no test panics while it holds the fake"))
    }
}

impl ProcessRunner for Handle<FakeProcessRunner> {
    fn run(&mut self, spec: &ProcessSpec<'_>) -> Result<ProcessResult, SeamError> {
        self.with(|runner| runner.run(spec))
    }
}

impl OutputFiles for Handle<FakeOutputFiles> {
    fn stat(&mut self, path: &str) -> Result<Option<FileKind>, SeamError> {
        self.with(|files| files.stat(path))
    }

    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, SeamError> {
        self.with(|files| files.list(path))
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, SeamError> {
        self.with(|files| files.read(path))
    }
}

/// How a [`Brittle`] reader goes wrong, on top of what its files do.
#[derive(Default)]
pub struct Faults {
    /// Every `read` fails with `Unknown`, whatever `stat` said.
    pub read_fails: bool,
    /// Every `list` fails with `Unknown`, whatever `stat` said.
    pub list_fails: bool,
    /// Every `list` answers these entries instead of what is there.
    pub listed: Option<Vec<DirEntry>>,
}

/// Files that fail where a tree that changes under a build system would.
pub struct Brittle {
    pub files: Handle<FakeOutputFiles>,
    pub faults: Faults,
}

impl OutputFiles for Brittle {
    fn stat(&mut self, path: &str) -> Result<Option<FileKind>, SeamError> {
        self.files.stat(path)
    }

    fn list(&mut self, path: &str) -> Result<Vec<DirEntry>, SeamError> {
        let entries = self.files.list(path)?;
        if self.faults.list_fails {
            return Err(SeamError::Unknown("the listing failed".to_owned()));
        }
        Ok(self.faults.listed.clone().unwrap_or(entries))
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, SeamError> {
        let bytes = self.files.read(path)?;
        if self.faults.read_fails {
            return Err(SeamError::Unknown("the read failed".to_owned()));
        }
        Ok(bytes)
    }
}

/// An entry of the store that keeps the writes it is given and can be made to refuse them.
#[derive(Default)]
pub struct Entry {
    pub written: Vec<(String, Vec<u8>)>,
    pub refuse: bool,
}

impl EntryWriter for Entry {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        if self.refuse {
            return Err(SeamError::Permanent("the entry refuses".to_owned()));
        }
        self.written.push((rel.to_owned(), bytes.to_vec()));
        Ok(())
    }
}

/// A build system as authored, one target and a stamp.
pub struct Case {
    pub system: Authored,
    pub target: Target,
    pub stamp: BuildStamp,
}

impl Case {
    fn new(kind: BuildSystemKind, binary: &str, config: &str, flag: &str, own: &str) -> Self {
        Self {
            system: Authored {
                kind: kind as i32,
                binary: binary.to_owned(),
                config_file: config.to_owned(),
                mode: "ci".to_owned(),
                flags: vec![flag.to_owned()],
                env: BTreeMap::from([("LOG".to_owned(), "warn".to_owned())]),
            },
            target: Target {
                label: LABEL.to_owned(),
                flags: vec![own.to_owned()],
                env: BTreeMap::new(),
            },
            stamp: BuildStamp {
                version: "1.2.3".to_owned(),
                build_number: 41,
                source_commit: "abc123".to_owned(),
                timestamp_ms: 1700,
            },
        }
    }

    pub fn buck2() -> Self {
        Self::new(
            BuildSystemKind::Buck2,
            "/opt/buck2",
            "/repo/.buckconfig.ci",
            "release.version={release_version}",
            "--local-only",
        )
    }

    pub fn bazel() -> Self {
        Self::new(
            BuildSystemKind::Bazel,
            "/opt/bazel",
            "/repo/.bazelrc",
            "--define=version={release_version}",
            "--keep_going",
        )
    }

    /// Where the report of a test invocation goes: the fixed file name of the kind.
    pub fn report(&self) -> String {
        let name = invocation::report_file_name(
            BuildSystemKind::try_from(self.system.kind).expect("a kind"),
        );
        format!("/scratch/t1/{}", name.expect("a build system"))
    }

    pub fn inv<'a>(&'a self, verb: Verb, report: Option<&'a str>) -> Invocation<'a> {
        Invocation {
            system: &self.system,
            target: &self.target,
            verb,
            report,
            stamp: &self.stamp,
            revision: "0123",
            platform: "linux-x86_64",
        }
    }

    /// The command of `verb`, from the layering of the design.
    pub fn command(&self, verb: Verb) -> Command {
        let report = self.report();
        let report = (verb == Verb::Test).then_some(report.as_str());
        invocation::command(&self.inv(verb, report)).expect("the case layers")
    }
}

/// `words` as the strings of an argument vector.
pub fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| (*w).to_owned()).collect()
}
