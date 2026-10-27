// SPDX-License-Identifier: Apache-2.0
//! A build system that makes its outputs from the layered command, so a changed flag,
//! environment value or stamp changes the bytes. The layering is the real one. It makes its
//! outputs and its reports itself and reads no files.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use ucdk_core::ManualClock;
use ucdk_release::test_report;
use ucdk_release_proto::v1::{BuildSystemKind, Reason};
use ucdk_release_seams::{
    BuildSystem, Command, EntryWriter, Heartbeat, Invocation, Located, SeamError, Shape,
    TestOutcome,
};

#[derive(Default)]
struct Faults {
    fail_at_call: Option<u32>,
    failure_tail: String,
    die_at_call: Option<u32>,
    unstartable: bool,
    no_output: BTreeSet<String>,
    two_outputs: BTreeSet<String>,
    wrong_shape: BTreeSet<String>,
    directory_output: BTreeSet<String>,
    symlink_in_output: BTreeSet<String>,
    output_changes_after_listing: BTreeSet<String>,
    mutate_earlier_output: bool,
    test_runs_nothing: BTreeSet<String>,
    test_fails: BTreeSet<String>,
    report_missing: BTreeSet<String>,
    report_unparseable: BTreeSet<String>,
    report_skipped: BTreeSet<String>,
    beats: Option<(u32, Duration)>,
    silent_for: Option<Duration>,
}

pub struct FakeBuildSystem {
    kind: BuildSystemKind,
    clock: ManualClock,
    faults: Faults,
    calls: u32,
    commands: Vec<Command>,
    outputs: BTreeMap<String, Vec<u8>>,
    /// The paths in the order they were first built.
    built: Vec<String>,
}

/// The directory and the file name an output of `label` is made at. The directory spells the
/// label with `_` as `__`, `:` as `_c` and `/` as `_s`: every character has one spelling and
/// no spelling starts another, so two labels never share a directory.
fn output_at(label: &str, suffix: &str) -> (String, String) {
    let name = label.rsplit([':', '/']).next().unwrap_or_default();
    let dir = label
        .replace('_', "__")
        .replace(':', "_c")
        .replace('/', "_s");
    (format!("/out/{dir}"), format!("{name}{suffix}"))
}

/// What a build of `label` with `command` writes: it names every argument and every
/// environment pair, so the bytes move with any of them. Each is quoted, so a line break or an
/// `=` inside one cannot pass for the end of it.
fn render(label: &str, command: &Command) -> Vec<u8> {
    let mut text = format!("built {label:?}\n");
    for arg in &command.argv {
        text.push_str(&format!("{arg:?}\n"));
    }
    for pair in &command.env {
        text.push_str(&format!("{pair:?}\n"));
    }
    text.into_bytes()
}

/// A report of `kind` whose results are all skipped, or of a target that was not built, so none
/// of them ran.
fn skipped_report(kind: BuildSystemKind) -> &'static [u8] {
    match kind {
        BuildSystemKind::Bazel => {
            br#"{"testSummary":{"overallStatus":"FAILED_TO_BUILD","totalRunCount":4}}"#
        }
        _ => br#"{"TestResult":{"status":"SKIP"}}"#,
    }
}

impl FakeBuildSystem {
    /// A build system of `kind` whose beats and silences move `clock`.
    pub fn new(kind: BuildSystemKind, clock: &ManualClock) -> Self {
        Self {
            kind,
            clock: clock.clone(),
            faults: Faults::default(),
            calls: 0,
            commands: Vec::new(),
            outputs: BTreeMap::new(),
            built: Vec::new(),
        }
    }

    // ---- faults ----

    /// The `n`th invocation (1 is the first, builds and tests together) exits nonzero.
    pub fn fail_at_call(&mut self, n: u32) {
        self.faults.fail_at_call = Some(n);
    }

    /// What an invocation that exits nonzero prints last; the `Permanent` answer of a build and
    /// the tail of a test that did not pass carry it, as the seam promises the tail of the
    /// output.
    pub fn failure_tail(&mut self, tail: &str) {
        self.faults.failure_tail = tail.to_owned();
    }

    /// The `n`th invocation is a build killed under the runner: `Unknown`.
    pub fn die_at_call(&mut self, n: u32) {
        self.faults.die_at_call = Some(n);
    }

    /// The binary cannot be started: `Unknown`.
    pub fn unstartable(&mut self) {
        self.faults.unstartable = true;
    }

    /// The build of `label` lists no output; the seam refuses it as `Refused(BUILD_OUTPUT)`.
    pub fn no_output(&mut self, label: &str) {
        self.faults.no_output.insert(label.to_owned());
    }

    /// The build of `label` lists two outputs; the seam refuses it as `Refused(BUILD_OUTPUT)`.
    pub fn two_outputs(&mut self, label: &str) {
        self.faults.two_outputs.insert(label.to_owned());
    }

    /// The output of `label` is a directory where a file is expected.
    pub fn wrong_shape(&mut self, label: &str) {
        self.faults.wrong_shape.insert(label.to_owned());
    }

    /// The output of `label` is a directory, as an image layout is.
    pub fn directory_output(&mut self, label: &str) {
        self.faults.directory_output.insert(label.to_owned());
    }

    /// The output of `label` holds a symlink.
    pub fn symlink_in_output(&mut self, label: &str) {
        self.faults.symlink_in_output.insert(label.to_owned());
    }

    /// The tree of `label` differs between the listing and `copy_out`.
    pub fn output_changes_after_listing(&mut self, label: &str) {
        self.faults
            .output_changes_after_listing
            .insert(label.to_owned());
    }

    /// A later build changes the tree of the first one.
    pub fn mutate_earlier_output(&mut self) {
        self.faults.mutate_earlier_output = true;
    }

    /// The test target `label` runs nothing.
    pub fn test_runs_nothing(&mut self, label: &str) {
        self.faults.test_runs_nothing.insert(label.to_owned());
    }

    /// The test target `label` runs and fails.
    pub fn test_fails(&mut self, label: &str) {
        self.faults.test_fails.insert(label.to_owned());
    }

    /// The report of the test target `label` is missing.
    pub fn report_missing(&mut self, label: &str) {
        self.faults.report_missing.insert(label.to_owned());
    }

    /// The report of the test target `label` cannot be parsed.
    pub fn report_unparseable(&mut self, label: &str) {
        self.faults.report_unparseable.insert(label.to_owned());
    }

    /// The report of the test target `label` holds only results that are skipped or untested.
    pub fn report_skipped_only(&mut self, label: &str) {
        self.faults.report_skipped.insert(label.to_owned());
    }

    /// Each invocation beats `n` times, moving the clock by `step` before each beat.
    pub fn beats(&mut self, n: u32, step: Duration) {
        self.faults.beats = Some((n, step));
    }

    /// Each invocation moves the clock by `by` and never beats.
    pub fn silent_for(&mut self, by: Duration) {
        self.faults.silent_for = Some(by);
    }

    /// Every command it was asked to run, in order.
    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    /// The part every invocation shares: layer the command and record it, fail as scripted,
    /// beat or stay silent.
    fn invoke(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<Command, SeamError> {
        self.calls += 1;
        let n = self.calls;
        let command = self.command(inv)?;
        self.commands.push(command.clone());
        let label = &inv.target.label;
        if self.faults.unstartable {
            return Err(SeamError::Unknown(
                "the binary cannot be started".to_owned(),
            ));
        }
        if self.faults.die_at_call == Some(n) {
            return Err(SeamError::Unknown("the build was killed".to_owned()));
        }
        let (beats, step) = self.faults.beats.unwrap_or_default();
        for _ in 0..beats {
            self.clock.advance(step);
            heartbeat
                .beat()
                .map_err(|_| SeamError::Refused(Reason::LeaseLost))?;
        }
        if let Some(by) = self.faults.silent_for {
            self.clock.advance(by);
        }
        if self.faults.fail_at_call == Some(n) {
            return Err(SeamError::Permanent(format!(
                "the invocation of {label} exited 1, and printed last: {}",
                self.faults.failure_tail
            )));
        }
        Ok(command)
    }
}

impl BuildSystem for FakeBuildSystem {
    fn kind(&self) -> BuildSystemKind {
        self.kind
    }

    fn build(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<Vec<Located>, SeamError> {
        let command = self.invoke(inv, heartbeat)?;
        let label = inv.target.label.as_str();
        if self.faults.symlink_in_output.contains(label) {
            return Err(SeamError::Refused(Reason::BuildOutput));
        }
        // The earliest built, not the smallest path.
        if self.faults.mutate_earlier_output
            && let Some(first) = self
                .built
                .first()
                .and_then(|path| self.outputs.get_mut(path))
        {
            first.extend_from_slice(b"mutated");
        }
        let shape = if self.faults.directory_output.contains(label)
            != self.faults.wrong_shape.contains(label)
        {
            Shape::Directory
        } else {
            Shape::File
        };
        let located = |suffix: &str| {
            let (dir, name) = output_at(label, suffix);
            Located {
                label: label.to_owned(),
                path: format!("{dir}/{name}"),
                shape,
            }
        };
        // The seam refuses a listing that is not exactly one output.
        if self.faults.no_output.contains(label) || self.faults.two_outputs.contains(label) {
            return Err(SeamError::Refused(Reason::BuildOutput));
        }
        let out = located("");
        if self
            .outputs
            .insert(out.path.clone(), render(label, &command))
            .is_none()
        {
            self.built.push(out.path.clone());
        }
        Ok(vec![out])
    }

    fn test(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<TestOutcome, SeamError> {
        self.invoke(inv, heartbeat)?;
        let label = inv.target.label.as_str();
        let report = if self.faults.report_missing.contains(label) {
            test_report::ran(self.kind, None)
        } else if self.faults.report_unparseable.contains(label) {
            test_report::ran(self.kind, Some(b"this is not a report"))
        } else if self.faults.report_skipped.contains(label) {
            test_report::ran(self.kind, Some(skipped_report(self.kind)))
        } else {
            u32::from(!self.faults.test_runs_nothing.contains(label))
        };
        let passed = report > 0 && !self.faults.test_fails.contains(label);
        Ok(TestOutcome {
            passed,
            ran: report,
            tail: if passed {
                format!("{label} passed")
            } else {
                format!(
                    "{label} did not pass, and printed last: {}",
                    self.faults.failure_tail
                )
            },
        })
    }

    fn copy_out(&mut self, out: &Located, into: &mut dyn EntryWriter) -> Result<(), SeamError> {
        let Some(bytes) = self.outputs.get(&out.path) else {
            return Err(SeamError::Permanent(format!("{} was not built", out.path)));
        };
        let mut bytes = bytes.clone();
        if self
            .faults
            .output_changes_after_listing
            .contains(&out.label)
        {
            bytes.extend_from_slice(b"changed");
        }
        let name = out.path.rsplit('/').next().unwrap_or_default();
        match out.shape {
            Shape::File => into.put_file(name, &bytes),
            Shape::Directory => into.put_file(&format!("{name}/layout"), &bytes),
        }
    }
}
