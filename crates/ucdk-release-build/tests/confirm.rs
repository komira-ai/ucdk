// SPDX-License-Identifier: Apache-2.0
//! The confirmation of `Buck2` and `Bazel` against the real build tools.
//!
//! The tests of the crate that run on every pull request script the tools. This file runs the
//! real ones, once, on request (`cargo xtask build-confirm`, which installs the pinned releases
//! and runs these tests with `--include-ignored`). Each test builds one genrule target of a
//! small fixture workspace (`tests/confirm/`) through `OsProcessRunner`, `LocalFiles` and the
//! adapter, and holds what the adapter finds to the file the genrule wrote.
//!
//! Every call of a tool goes through a runner that keeps the raw transcript of the call under
//! `transcripts/` (next to this file's scratch directory in the target directory): the command
//! line, the exit, the standard output and the tail of the standard error. The workflow uploads
//! that directory, so a red run and a green one both leave the evidence behind.
//!
//! Two points of the design were unverified until these tests ran, and each test prints what it
//! saw (the lines that start with `OBSERVED`):
//!
//! - whether `bazel info execution_root` accepts the flags of the build (`--keep_going`), which
//!   the adapter passes to it;
//! - whether the label that `buck2 build --show-full-output` prints carries a suffix after the
//!   target (the configuration), which the adapter's line parser would not read.

use std::collections::BTreeMap;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ucdk_release::invocation::{Command, Invocation, Verb};
use ucdk_release_build::{Bazel, Buck2, LocalFiles, OsProcessRunner};
use ucdk_release_proto::v1::{BuildStamp, BuildSystem as Authored, BuildSystemKind, Target};
use ucdk_release_seams::{
    BuildSystem, EntryWriter, Exit, Heartbeat, Located, ProcessResult, ProcessRunner, ProcessSpec,
    SeamError, Shape, Stop,
};

/// The label of the one target of both fixtures, and the name and bytes of the file it writes.
const LABEL: &str = "//:hello";
const FILE: &str = "hello.txt";
const BYTES: &[u8] = b"hello\n";

/// How long a test waits for a call that should return before it reports the call as hung: a
/// guard against a build tool that leaves a process holding the output open, which would keep
/// the runner waiting for ever. No result depends on how long anything took.
const HANG_GUARD: Duration = Duration::from_secs(900);

fn scratch() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("build-confirm")
}

fn fixture(tool: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/confirm")
        .join(tool)
}

/// Runs `f` on a thread of its own; if it has not returned after `HANG_GUARD` the test fails
/// and says what `what` was.
fn guarded(what: &'static str, f: impl FnOnce() + Send + 'static) {
    let (done, finished) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(catch_unwind(AssertUnwindSafe(f)));
    });
    match finished.recv_timeout(HANG_GUARD) {
        Ok(Ok(())) => {}
        Ok(Err(panic)) => resume_unwind(panic),
        Err(_) => panic!("{what} did not return"),
    }
}

struct Never;

impl Heartbeat for Never {
    fn beat(&self) -> Result<(), Stop> {
        Ok(())
    }
}

#[derive(Default)]
struct Entry {
    written: Vec<(String, Vec<u8>)>,
}

impl EntryWriter for Entry {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        self.written.push((rel.to_owned(), bytes.to_vec()));
        Ok(())
    }
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("a directory");
    for entry in fs::read_dir(from).expect("the fixture") {
        let entry = entry.expect("an entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("a copy");
        }
    }
}

/// A copy of the fixture of `tool` in a directory of its own, and the directory the raw
/// transcripts of the tool go to (empty).
struct Work {
    root: String,
    /// A directory beside the workspace for the tool's own state, which a tool may not keep in
    /// the workspace it builds.
    state: String,
    transcripts: PathBuf,
}

impl Work {
    fn new(tool: &str) -> Self {
        let base = scratch();
        let work = base.join("work").join(tool);
        let transcripts = base.join("transcripts");
        let _ = fs::remove_dir_all(&work);
        copy_tree(&fixture(tool), &work);
        fs::create_dir_all(&transcripts).expect("transcripts");
        let state = base.join("work").join(format!("{tool}-state"));
        let _ = fs::remove_dir_all(&state);
        fs::create_dir_all(&state).expect("the state directory");
        let path = |dir: &Path| {
            fs::canonicalize(dir)
                .expect("a directory")
                .to_string_lossy()
                .into_owned()
        };
        Self {
            root: path(&work),
            state: path(&state),
            transcripts,
        }
    }
}

impl Work {
    /// The reader of the files the tool wrote: confined to the workspace and the state directory.
    fn files(&self) -> LocalFiles {
        LocalFiles::within([&self.root, &self.state]).expect("the directories resolve")
    }
}

/// The environment a tool gets: the variables it cannot start without, from this process.
fn tool_env() -> BTreeMap<String, String> {
    ["PATH", "HOME", "USER", "TMPDIR"]
        .into_iter()
        .filter_map(|name| Some((name.to_owned(), std::env::var(name).ok()?)))
        .collect()
}

fn authored(kind: BuildSystemKind, binary: &str, config_file: &str) -> Authored {
    Authored {
        kind: kind as i32,
        binary: binary.to_owned(),
        config_file: config_file.to_owned(),
        mode: String::new(),
        flags: Vec::new(),
        env: tool_env(),
    }
}

fn target(flags: &[&str]) -> Target {
    Target {
        label: LABEL.to_owned(),
        flags: flags.iter().map(|f| (*f).to_owned()).collect(),
        env: BTreeMap::new(),
    }
}

fn stamp() -> BuildStamp {
    BuildStamp {
        version: "1.0.0".to_owned(),
        build_number: 1,
        source_commit: "0".repeat(40),
        timestamp_ms: 1,
    }
}

/// Keeps the transcript of each call, then answers what the real runner answered.
struct Recorder {
    inner: OsProcessRunner,
    dir: PathBuf,
    prefix: &'static str,
    calls: usize,
}

impl Recorder {
    fn new(dir: &Path, prefix: &'static str) -> Self {
        Self {
            inner: OsProcessRunner,
            dir: dir.to_owned(),
            prefix,
            calls: 0,
        }
    }
}

fn exit_text(exit: Exit) -> String {
    match exit {
        Exit::Code(code) => format!("exit {code}"),
        Exit::Signal(signal) => format!("signal {signal}"),
    }
}

fn record(dir: &Path, stem: &str, command: &Command, result: &ProcessResult) {
    let write = |suffix: &str, bytes: &[u8]| {
        fs::write(dir.join(format!("{stem}.{suffix}")), bytes).expect("a transcript");
    };
    write("argv", command.argv.join("\n").as_bytes());
    write("exit", exit_text(result.exit).as_bytes());
    write("stdout", &result.stdout);
    write("stderr", &result.stderr_tail);
    println!(
        "OBSERVED {stem}: {} -> {}; standard output {:?}",
        command.argv.join(" "),
        exit_text(result.exit),
        String::from_utf8_lossy(&result.stdout)
    );
}

impl ProcessRunner for Recorder {
    fn run(&mut self, spec: &ProcessSpec<'_>) -> Result<ProcessResult, SeamError> {
        self.calls += 1;
        // The word of the call is the first one after the program that is not an option.
        let verb = spec
            .command
            .argv
            .iter()
            .skip(1)
            .find(|word| !word.starts_with('-'))
            .map_or("", String::as_str);
        let stem = format!("{}-{:02}-{verb}", self.prefix, self.calls);
        // The command line first, so a call that never returns still leaves it behind.
        fs::write(
            self.dir.join(format!("{stem}.argv")),
            spec.command.argv.join("\n"),
        )
        .expect("a transcript");
        let result = self.inner.run(spec)?;
        record(&self.dir, &stem, spec.command, &result);
        Ok(result)
    }
}

/// The version the tool says it is, kept as a transcript of its own.
fn record_version(work: &Work, prefix: &str, binary: &str) {
    let command = Command {
        argv: vec![binary.to_owned(), "--version".to_owned()],
        env: tool_env().into_iter().collect(),
    };
    let result = OsProcessRunner
        .run(&ProcessSpec {
            command: &command,
            cwd: &work.root,
            heartbeat: &Never,
            stdout_limit: 1 << 16,
        })
        .expect("the tool starts");
    record(
        &work.transcripts,
        &format!("{prefix}-00-version"),
        &command,
        &result,
    );
    assert_eq!(result.exit, Exit::Code(0), "{binary} --version");
}

fn text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// What the adapter found, held to the file the genrule wrote: one file, the label asked for,
/// the bytes of the genrule, and the name the copy leaves in the entry.
fn assert_found(system: &mut dyn BuildSystem, located: &[Located]) -> String {
    let [found] = located else {
        panic!("one output expected, found {located:?}");
    };
    assert_eq!(found.label, LABEL);
    assert_eq!(found.shape, Shape::File);
    assert!(found.path.ends_with(&format!("/{FILE}")), "{}", found.path);
    let mut entry = Entry::default();
    system.copy_out(found, &mut entry).expect("the copy");
    assert_eq!(entry.written, [(FILE.to_owned(), BYTES.to_vec())]);
    assert_eq!(fs::read(&found.path).expect("the output"), BYTES);
    found.path.clone()
}

#[test]
#[ignore = "runs the real Buck2; `cargo xtask build-confirm` runs it"]
fn buck2_builds_the_fixture_target_and_the_adapter_finds_the_file() {
    guarded("the Buck2 confirmation", || {
        let work = Work::new("buck2");
        record_version(&work, "buck2", "buck2");
        let system = authored(BuildSystemKind::Buck2, "buck2", "");
        let target = target(&[]);
        let stamp = stamp();
        let inv = Invocation {
            system: &system,
            target: &target,
            verb: Verb::Build,
            report: None,
            stamp: &stamp,
            revision: "0",
            platform: "linux-x86_64",
        };
        let mut buck2 = Buck2::new(
            Recorder::new(&work.transcripts, "buck2"),
            work.files(),
            work.root.clone(),
        );
        let outcome = buck2.build(&inv, &Never);

        // The listing, as the tool printed it: the label, one space, the absolute path. What
        // follows the label is told before the adapter's answer is held to anything.
        let listing = text(&work.transcripts.join("buck2-01-build.stdout"));
        let line = listing.lines().next().unwrap_or_default();
        let (label, listed) = line.split_once(' ').unwrap_or_default();
        println!("OBSERVED buck2 label as printed: {label:?}; the rest of the line: {listed:?}");
        println!(
            "OBSERVED buck2 configuration suffix after the label: {}",
            if listed.starts_with('/') {
                "none"
            } else {
                "present"
            }
        );
        let located = outcome.expect("the build is found");
        let path = assert_found(&mut buck2, &located);
        assert_eq!(listing.lines().count(), 1, "one line for one output");
        assert_eq!(listed, path);
        assert!(
            path.starts_with(&format!("{}/buck-out/", work.root)),
            "{path}"
        );
    });
}

#[test]
#[ignore = "runs the real Bazel; `cargo xtask build-confirm` runs it"]
fn bazel_builds_the_fixture_target_and_the_adapter_finds_the_file() {
    guarded("the Bazel confirmation", || {
        let work = Work::new("bazel");
        record_version(&work, "bazel", "bazel");
        // Bazel keeps its output tree in the state directory, which the reader may read and the
        // workspace does not hold; the configuration file of the machine says so.
        let rc = format!("{}/confirm.bazelrc", work.root);
        fs::write(
            &rc,
            format!("startup --output_user_root={}/output\n", work.state),
        )
        .expect("the configuration file");
        let system = authored(BuildSystemKind::Bazel, "bazel", &rc);
        // The flag of the target goes to the build, to `cquery` and to `info` alike.
        let target = target(&["--keep_going"]);
        let stamp = stamp();
        let inv = Invocation {
            system: &system,
            target: &target,
            verb: Verb::Build,
            report: None,
            stamp: &stamp,
            revision: "0",
            platform: "linux-x86_64",
        };
        let mut bazel = Bazel::new(
            Recorder::new(&work.transcripts, "bazel"),
            work.files(),
            work.root.clone(),
        );
        let outcome = bazel.build(&inv, &Never);
        let said = |suffix: &str| {
            fs::read_to_string(work.transcripts.join(format!("bazel-03-info.{suffix}")))
                .unwrap_or_else(|_| "not run".to_owned())
        };
        let (info_exit, info_argv) = (said("exit"), said("argv"));
        println!(
            "OBSERVED bazel info with --keep_going: {} ({})",
            match info_exit.as_str() {
                "exit 0" => "accepted",
                "not run" => "not run",
                _ => "refused",
            },
            info_argv.replace('\n', " ")
        );
        let located = outcome.expect("the build is found");
        assert!(info_argv.lines().any(|a| a == "--keep_going"));
        let path = assert_found(&mut bazel, &located);

        // The two listings, as the tool printed them, joined as the design says.
        let relative = text(&work.transcripts.join("bazel-02-cquery.stdout"));
        let root = text(&work.transcripts.join("bazel-03-info.stdout"));
        assert_eq!(
            path,
            format!("{}/{}", root.trim_end(), relative.trim_end()),
            "execution root and relative path"
        );
        assert!(path.contains("/bazel-out/"), "{path}");
    });
}

#[test]
fn the_fixtures_declare_the_target_that_the_tests_build() {
    let buck = text(&fixture("buck2").join("BUCK"));
    let bazel = text(&fixture("bazel").join("BUILD.bazel"));
    for build_file in [&buck, &bazel] {
        assert!(build_file.contains("name = \"hello\""), "{build_file}");
        assert!(build_file.contains(FILE), "{build_file}");
        assert!(build_file.contains("echo hello >"), "{build_file}");
    }
    assert_eq!(BYTES, b"hello\n");
    for needed in [".buckroot", ".buckconfig", "toolchains/BUCK"] {
        assert!(fixture("buck2").join(needed).is_file(), "{needed}");
    }
    assert!(fixture("bazel").join("MODULE.bazel").is_file());
}
