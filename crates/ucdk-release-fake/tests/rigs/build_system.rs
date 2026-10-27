// SPDX-License-Identifier: Apache-2.0
//! The build-system rig: the fake build system in each world, and the stubs that break one
//! promise of a build system in one world.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::time::Duration;

use ucdk_core::{Clock, ManualClock};
use ucdk_release_fake::FakeBuildSystem;
use ucdk_release_proto::v1::{BuildSystem as Authored, BuildSystemKind, Reason, Target};
use ucdk_release_seams::kit::{BUILD_LABEL, BuildSystemRig, BuildWorld};
use ucdk_release_seams::{
    BuildSystem, Command, EntryWriter, Heartbeat, Invocation, Located, SeamError, Shape, Stop,
    TestOutcome, Verb,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// The bytes of a build carry the number of builds this instance made.
    CallCounterInTheBytes,
    /// The target's flags come before the base flags.
    TargetFlagsFirst,
    /// The base value of a key wins over the target's.
    BaseEnvironmentWins,
    /// The environment holds a variable of the process.
    InheritsTheEnvironment,
    /// A placeholder of a flag that holds the revision is left as written.
    LeavesTheRevisionPlaceholder,
    /// The build number of the stamp is added as a flag nobody wrote.
    StampLeaksIntoTheCommand,
    /// The flag ucdk owns comes before the author's flags.
    OwnedFlagFirst,
    /// A build lists its one output twice.
    ListsTheOutputTwice,
    /// A build that lists nothing is an empty answer.
    NoOutputIsAnEmptyListing,
    /// A build that lists two outputs answers the first.
    TwoOutputsAnswersTheFirst,
    /// A build whose output holds a symlink answers the output.
    SymlinkIsAccepted,
    /// A failed build is `Permanent` without the tail of its output.
    FailureHasNoTail,
    /// A build system that cannot start is `Permanent`.
    UnstartableIsPermanent,
    /// The bytes of a build carry the time of the run.
    ClockInTheBytes,
    /// `copy_out` also writes a path that climbs out of the entry.
    CopyOutEscapes,
    /// A test that ran nothing passes.
    NothingRanPasses,
    /// A test that failed passes.
    FailureIsAPass,
    /// A test whose report cannot be parsed passes.
    UnparseableReportPasses,
    /// A `Stop` is not forwarded.
    StopIsSwallowed,
    /// `copy_out` writes `garbage` in place of the output's bytes.
    CopyOutWritesGarbage,
    /// `copy_out` writes an empty file in place of the output's bytes.
    CopyOutWritesAnEmptyFile,
    /// `copy_out` writes a second file the output does not hold.
    CopyOutWritesAnExtraFile,
    /// A `Test` command inherits a variable of the process.
    TestVerbInheritsTheEnvironment,
    /// A `Test` command lets the base value of a key win over the target's.
    TestVerbBaseEnvironmentWins,
    /// A `Test` command puts the target's flags before the base flags.
    TestVerbTargetFlagsFirst,
    /// A `Test` command carries no environment at all.
    TestVerbDropsTheEnvironment,
    /// A Bazel `Query` command inherits a variable of the process.
    QueryVerbInheritsTheEnvironment,
    /// A Bazel `Info` command carries the build number of the stamp as a flag nobody wrote.
    InfoVerbLeaksTheStamp,
    /// A directory output is listed as a file.
    DirectoryIsListedAsAFile,
    /// A build whose output is a directory is refused.
    DirectoryOutputIsRefused,
    /// A build whose output is a directory with a symlink below its top level answers the
    /// directory.
    NestedSymlinkIsAccepted,
    /// `copy_out` of a directory also writes a path that climbs out of the entry.
    CopyOutOfADirectoryEscapes,
    /// A `Test` command repeats the base flags after the flag ucdk owns.
    BaseFlagsRepeatedAfterTheOwnedFlag,
    /// `copy_out` of a directory writes `garbage` in place of the bytes of its files.
    CopyOutOfADirectoryWritesGarbage,
    /// The bytes `copy_out` writes for a directory carry the time of the run.
    CopyOutOfADirectoryCarriesTheClock,
    /// A test runs a command other than the one it layers (the flags reversed, the base flags
    /// gone, a variable of the process added).
    TestRunsOtherCommand,
    /// A Bazel build also runs a query with a variable of the process that it does not layer.
    QueryRunsOtherCommand,
    /// A Bazel build also runs an info call with a flag that it does not layer.
    InfoRunsOtherCommand,
    /// A command that is not a build loses its last argument (the label or the tail).
    DropsTheLabel,
    /// A command that is not a build says `build` where its verb word is.
    WrongVerbWord,
    /// A `Test` command names its report path without the flag.
    BareReportFlag,
    /// A `Test` command names its report with the other kind's flag.
    SwappedReportFlagNames,
    /// A `Test` command names its report twice.
    DuplicatedReportFlag,
    /// The config file and the mode are ignored.
    IgnoresConfigAndMode,
    /// A flag passed again is dropped the second time.
    DedupsFlags,
    /// A placeholder that does not exist, or a stray brace, passes through as written.
    UnknownPlaceholderPassedThrough,
    /// A doubled brace stays doubled.
    NoBraceUnescape,
    /// Only `{release_version}` is replaced in an environment value.
    EnvOnlyVersionPlaceholder,
    /// `{platform}` is left as written in a base flag.
    BaseFlagPlatformUnsubstituted,
    /// A `Test` command without a report path has a layering.
    TestWithoutReportAccepted,
    /// The revision is added as a flag nobody wrote to every command that is not a build.
    LeaksTheRevision,
    /// A test says it ran seven times what the report says.
    RanInflated,
    /// The tail of a failing test is not what it printed.
    TestTailGarbage,
    /// A report that holds only skipped results counts them as runs.
    SkippedCountedAsRan,
    /// The heartbeat gets no beat after the third.
    CapsBeatsAtThree,
    /// A `Stop` is answered with more beats, and with `LEASE_LOST` once the work is done.
    KeepsBeatingAfterStop,
    /// A test of a build system that cannot start is `Permanent`.
    UnstartableTestIsPermanent,
    /// A failed build answers all of its output, the head and then the tail.
    FailurePadded,
}

/// The time on the clock of a rig before anything moves it.
const START_MS: u64 = 1_000_000;

/// Where the fake makes the output of [`BUILD_LABEL`]: the directory spells the label with
/// `_` as `__`, `:` as `_c` and `/` as `_s`, so `//pkg:out` is made in `/out/_s_spkg_cout`.
const OUTPUT_PATH: &str = "/out/_s_spkg_cout/out";

pub struct FakeBuildRig {
    pub clock: ManualClock,
    pub kind: BuildSystemKind,
    pub flaw: Option<Flaw>,
}

impl FakeBuildRig {
    pub fn conforming(kind: BuildSystemKind) -> Self {
        Self {
            clock: ManualClock::at(START_MS),
            kind,
            flaw: None,
        }
    }

    pub fn flawed(kind: BuildSystemKind, flaw: Flaw) -> Self {
        Self {
            flaw: Some(flaw),
            ..Self::conforming(kind)
        }
    }
}

pub struct Flawed {
    inner: FakeBuildSystem,
    flaw: Option<Flaw>,
    world: BuildWorld,
    clock: ManualClock,
    builds: u32,
    /// The commands run, as the process under the build system was given them.
    ran: Vec<Command>,
    /// How many commands of the fake `ran` already holds.
    synced: usize,
}

impl BuildSystemRig for FakeBuildRig {
    type System = Flawed;

    fn system(&self, world: &BuildWorld) -> Flawed {
        let mut fake = FakeBuildSystem::new(self.kind, &self.clock);
        match world {
            BuildWorld::Normal => {}
            BuildWorld::NoOutput => fake.no_output(BUILD_LABEL),
            BuildWorld::TwoOutputs => fake.two_outputs(BUILD_LABEL),
            BuildWorld::SymlinkInOutput => fake.symlink_in_output(BUILD_LABEL),
            BuildWorld::DirectoryOutput => fake.directory_output(BUILD_LABEL),
            BuildWorld::SymlinkInDirectory => {
                fake.directory_output(BUILD_LABEL);
                fake.symlink_in_output(BUILD_LABEL);
            }
            BuildWorld::BuildFails { tail, .. } => {
                fake.fail_at_call(1);
                fake.failure_tail(tail);
            }
            BuildWorld::Unstartable => fake.unstartable(),
            BuildWorld::TestRunsNothing => fake.test_runs_nothing(BUILD_LABEL),
            BuildWorld::TestSkipsEverything => fake.report_skipped_only(BUILD_LABEL),
            BuildWorld::TestFails { tail } => {
                fake.test_fails(BUILD_LABEL);
                fake.failure_tail(tail);
            }
            BuildWorld::ReportMissing => fake.report_missing(BUILD_LABEL),
            BuildWorld::ReportUnparseable => fake.report_unparseable(BUILD_LABEL),
            BuildWorld::Progress { events } => fake.beats(*events, Duration::from_millis(1)),
        }
        Flawed {
            inner: fake,
            flaw: self.flaw,
            world: world.clone(),
            clock: self.clock.clone(),
            builds: 0,
            ran: Vec::new(),
            synced: 0,
        }
    }

    fn later(&self) {
        self.clock.advance(Duration::from_secs(86_400));
    }

    /// The fake writes one file, `out` (`out/layout` for a directory output), that names the
    /// label, then every argument and every environment pair of the command that built it, each
    /// on a line of its own and quoted as `Debug` quotes it.
    fn output_of(&self, inv: &Invocation<'_>, world: &BuildWorld) -> Vec<(String, Vec<u8>)> {
        let command = FakeBuildSystem::new(self.kind, &self.clock)
            .command(inv)
            .expect("the fake layers the invocation");
        let mut text = format!("built {BUILD_LABEL:?}\n");
        for arg in &command.argv {
            text.push_str(&format!("{arg:?}\n"));
        }
        for pair in &command.env {
            text.push_str(&format!("{pair:?}\n"));
        }
        let path = if *world == BuildWorld::DirectoryOutput {
            "out/layout"
        } else {
            "out"
        };
        vec![(path.to_owned(), text.into_bytes())]
    }

    fn executed(&self, system: &Flawed) -> Vec<Command> {
        system.ran.clone()
    }
}

/// How the heartbeat the build system was given reaches the fake.
enum Beating {
    Forwarded,
    /// Nothing is forwarded.
    Swallowed,
    /// Only the first `n` beats are forwarded.
    Capped(u32),
    /// Every beat is forwarded and a `Stop` is remembered, not obeyed.
    KeepsGoing,
}

/// The heartbeat the fake sees: the caller's, tampered with as `how` says.
struct Beat<'a> {
    heartbeat: &'a dyn Heartbeat,
    how: Beating,
    given: Cell<u32>,
    stopped: Cell<bool>,
}

impl Heartbeat for Beat<'_> {
    fn beat(&self) -> Result<(), Stop> {
        self.given.set(self.given.get() + 1);
        match self.how {
            Beating::Forwarded => self.heartbeat.beat(),
            Beating::Swallowed => Ok(()),
            Beating::Capped(n) if self.given.get() > n => Ok(()),
            Beating::Capped(_) => self.heartbeat.beat(),
            Beating::KeepsGoing => {
                if self.heartbeat.beat().is_err() {
                    self.stopped.set(true);
                }
                Ok(())
            }
        }
    }
}

/// An entry writer that adds `suffix` to what it is given.
struct Suffixed<'a> {
    into: &'a mut dyn EntryWriter,
    suffix: Vec<u8>,
    escape: bool,
    /// Written in place of the bytes it is given.
    replace: Option<Vec<u8>>,
    /// A second file written after each file.
    extra: bool,
}

impl EntryWriter for Suffixed<'_> {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        if self.escape {
            let _ = self.into.put_file("../escape", b"x");
        }
        let mut bytes = self.replace.clone().unwrap_or_else(|| bytes.to_vec());
        bytes.extend_from_slice(&self.suffix);
        self.into.put_file(rel, &bytes)?;
        if self.extra {
            self.into.put_file("extra", b"not in the output")?;
        }
        Ok(())
    }
}

/// The report flag of each kind, as the prefix of the argument that names the path.
const REPORT_FLAGS: [&str; 2] = ["--event-log=", "--build_event_json_file="];

/// Puts `flag` among the flags of the authors, after the second base flag, where the checks of
/// what comes before and after them do not look.
fn insert_among_flags(command: &mut Command, flag: String) {
    let at = command
        .argv
        .iter()
        .position(|a| a == "--base-two")
        .map_or(command.argv.len(), |i| i + 1);
    command.argv.insert(at, flag);
}

impl Flawed {
    fn is(&self, flaw: Flaw) -> bool {
        self.flaw == Some(flaw)
    }

    fn in_world(&self, flaw: Flaw, world: &BuildWorld) -> bool {
        self.is(flaw) && self.world == *world
    }

    /// The system and target the build system layers (`running` false) or runs (`running`
    /// true) for `inv`.
    fn seen(&self, inv: &Invocation<'_>, running: bool) -> (Authored, Target) {
        let mut system = inv.system.clone();
        let mut target = inv.target.clone();
        if self.is(Flaw::IgnoresConfigAndMode) {
            system.config_file.clear();
            system.mode.clear();
        }
        if running && self.is(Flaw::TestRunsOtherCommand) && inv.verb == Verb::Test {
            system.flags.clear();
            target.flags.reverse();
            system.env.insert("HOME".to_owned(), "/root".to_owned());
        }
        (system, target)
    }

    /// How the heartbeat of a call reaches the fake in this world.
    fn tampered<'a>(&self, heartbeat: &'a dyn Heartbeat) -> Beat<'a> {
        let how = if matches!(self.world, BuildWorld::Progress { .. }) {
            match self.flaw {
                Some(Flaw::StopIsSwallowed) => Beating::Swallowed,
                Some(Flaw::CapsBeatsAtThree) => Beating::Capped(3),
                Some(Flaw::KeepsBeatingAfterStop) => Beating::KeepsGoing,
                _ => Beating::Forwarded,
            }
        } else {
            Beating::Forwarded
        };
        Beat {
            heartbeat,
            how,
            given: Cell::new(0),
            stopped: Cell::new(false),
        }
    }

    /// Records the commands the fake ran since the last call, with the flaws of the layering.
    fn sync(&mut self, inv: &Invocation<'_>) {
        let fresh: Vec<Command> = self.inner.commands()[self.synced..].to_vec();
        self.synced += fresh.len();
        for mut command in fresh {
            self.flaw_command(&mut command, inv);
            self.ran.push(command);
        }
    }

    /// The flaws of the layering, applied to a layered command of `inv`.
    fn flaw_command(&self, command: &mut Command, inv: &Invocation<'_>) {
        // A flaw of one verb is the flaw of every verb, applied to that verb alone.
        let flaw = match (self.flaw, inv.verb) {
            (Some(Flaw::TestVerbInheritsTheEnvironment), Verb::Test)
            | (Some(Flaw::QueryVerbInheritsTheEnvironment), Verb::Query) => {
                Some(Flaw::InheritsTheEnvironment)
            }
            (Some(Flaw::TestVerbBaseEnvironmentWins), Verb::Test) => {
                Some(Flaw::BaseEnvironmentWins)
            }
            (Some(Flaw::TestVerbTargetFlagsFirst), Verb::Test) => Some(Flaw::TargetFlagsFirst),
            (Some(Flaw::BaseFlagsRepeatedAfterTheOwnedFlag), Verb::Test) => {
                command.argv.push("--base-one".to_owned());
                None
            }
            (Some(Flaw::InfoVerbLeaksTheStamp), Verb::Info) => Some(Flaw::StampLeaksIntoTheCommand),
            (Some(Flaw::TestVerbDropsTheEnvironment), Verb::Test) => {
                command.env.clear();
                None
            }
            (flaw, _) => flaw,
        };
        match flaw {
            Some(Flaw::TargetFlagsFirst) => {
                let at: Vec<usize> = (0..command.argv.len())
                    .filter(|i| {
                        command.argv[*i].starts_with("--base")
                            || command.argv[*i].starts_with("--target")
                    })
                    .collect();
                let flags: Vec<String> =
                    at.iter().rev().map(|i| command.argv[*i].clone()).collect();
                for (i, flag) in at.into_iter().zip(flags) {
                    command.argv[i] = flag;
                }
            }
            Some(Flaw::BaseEnvironmentWins) => {
                for (key, value) in &mut command.env {
                    if key == "X" {
                        "base".clone_into(value);
                    }
                }
            }
            Some(Flaw::InheritsTheEnvironment) => {
                command.env.push(("HOME".to_owned(), "/root".to_owned()));
                command.env.sort();
            }
            Some(Flaw::LeavesTheRevisionPlaceholder) => {
                for arg in &mut command.argv {
                    if arg.contains('@') {
                        *arg = arg.replace("0123", "{revision}");
                    }
                }
            }
            Some(Flaw::StampLeaksIntoTheCommand) => {
                insert_among_flags(
                    command,
                    format!("--build-number={}", inv.stamp.build_number),
                );
            }
            Some(Flaw::OwnedFlagFirst) => {
                if let Some(at) = command.argv.iter().position(|a| a.contains("/scratch/")) {
                    let owned = command.argv.remove(at);
                    command.argv.insert(2, owned);
                }
            }
            Some(Flaw::DedupsFlags) => {
                let mut seen = BTreeSet::new();
                command
                    .argv
                    .retain(|a| !a.starts_with("--") || a.len() == 2 || seen.insert(a.clone()));
            }
            Some(Flaw::NoBraceUnescape) => {
                for arg in &mut command.argv {
                    *arg = arg.replace("{x}", "{{x}}");
                }
                for (_, value) in &mut command.env {
                    *value = value.replace("{e}", "{{e}}");
                }
            }
            Some(Flaw::EnvOnlyVersionPlaceholder) => {
                for (key, value) in &mut command.env {
                    let raw = inv.target.env.get(key).or_else(|| inv.system.env.get(key));
                    if let Some(raw) = raw {
                        *value = raw.replace("{release_version}", &inv.stamp.version);
                    }
                }
            }
            Some(Flaw::BaseFlagPlatformUnsubstituted) => {
                for arg in &mut command.argv {
                    if arg == "--p=linux-x86_64" {
                        "--p={platform}".clone_into(arg);
                    }
                }
            }
            Some(Flaw::LeaksTheRevision) if inv.verb != Verb::Build => {
                insert_among_flags(command, format!("--revision={}", inv.revision));
            }
            Some(Flaw::DropsTheLabel) if inv.verb != Verb::Build => {
                command.argv.pop();
            }
            Some(Flaw::WrongVerbWord) if inv.verb != Verb::Build => {
                let word = match inv.verb {
                    Verb::Query => "cquery",
                    Verb::Info => "info",
                    _ => "test",
                };
                if let Some(arg) = command.argv.iter_mut().find(|a| a.as_str() == word) {
                    "build".clone_into(arg);
                }
            }
            Some(Flaw::BareReportFlag) if inv.verb == Verb::Test => {
                for arg in &mut command.argv {
                    for flag in REPORT_FLAGS {
                        if let Some(path) = arg.strip_prefix(flag) {
                            *arg = path.to_owned();
                            break;
                        }
                    }
                }
            }
            Some(Flaw::SwappedReportFlagNames) if inv.verb == Verb::Test => {
                for arg in &mut command.argv {
                    for (flag, other) in [
                        (REPORT_FLAGS[0], REPORT_FLAGS[1]),
                        (REPORT_FLAGS[1], REPORT_FLAGS[0]),
                    ] {
                        if let Some(path) = arg.strip_prefix(flag) {
                            *arg = format!("{other}{path}");
                            break;
                        }
                    }
                }
            }
            Some(Flaw::DuplicatedReportFlag) if inv.verb == Verb::Test => {
                let at = command
                    .argv
                    .iter()
                    .position(|a| REPORT_FLAGS.iter().any(|f| a.starts_with(f)));
                if let Some(at) = at {
                    let again = command.argv[at].clone();
                    command.argv.insert(at, again);
                }
            }
            _ => {}
        }
    }
}

impl BuildSystem for Flawed {
    fn kind(&self) -> BuildSystemKind {
        self.inner.kind()
    }

    fn command(&self, inv: &Invocation<'_>) -> Result<Command, SeamError> {
        let (system, target) = self.seen(inv, false);
        let inv = Invocation {
            system: &system,
            target: &target,
            ..*inv
        };
        let layered = self.inner.command(&inv);
        let report_fits = inv.report.is_some() == (inv.verb == Verb::Test);
        let accepts = (self.is(Flaw::TestWithoutReportAccepted)
            && inv.verb == Verb::Test
            && inv.report.is_none())
            || (self.is(Flaw::UnknownPlaceholderPassedThrough) && report_fits);
        match layered {
            Err(_) if accepts => Ok(Command {
                argv: Vec::new(),
                env: Vec::new(),
            }),
            other => {
                let mut command = other?;
                self.flaw_command(&mut command, &inv);
                Ok(command)
            }
        }
    }

    fn build(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<Vec<Located>, SeamError> {
        self.builds += 1;
        let (system, target) = self.seen(inv, true);
        let run = Invocation {
            system: &system,
            target: &target,
            ..*inv
        };
        let beat = self.tampered(heartbeat);
        let result = self.inner.build(&run, &beat);
        let result = if beat.stopped.get() {
            Err(SeamError::Refused(Reason::LeaseLost))
        } else {
            result
        };
        self.sync(inv);
        if matches!(
            self.flaw,
            Some(Flaw::QueryRunsOtherCommand | Flaw::InfoRunsOtherCommand)
        ) && self.inner.kind() == BuildSystemKind::Bazel
            && result.is_ok()
        {
            let verb = if self.is(Flaw::QueryRunsOtherCommand) {
                Verb::Query
            } else {
                Verb::Info
            };
            let mut extra = self
                .inner
                .command(&Invocation {
                    verb,
                    report: None,
                    ..*inv
                })
                .expect("the fake layers the invocation");
            if verb == Verb::Query {
                extra.env.push(("HOME".to_owned(), "/root".to_owned()));
                extra.env.sort();
            } else {
                insert_among_flags(&mut extra, "--extra".to_owned());
            }
            self.ran.push(extra);
        }
        let made = Located {
            label: BUILD_LABEL.to_owned(),
            path: OUTPUT_PATH.to_owned(),
            shape: Shape::File,
        };
        match (self.flaw, result) {
            (Some(Flaw::DirectoryIsListedAsAFile), Ok(mut located))
                if self.world == BuildWorld::DirectoryOutput =>
            {
                located[0].shape = Shape::File;
                Ok(located)
            }
            (Some(Flaw::DirectoryOutputIsRefused), Ok(_))
                if self.world == BuildWorld::DirectoryOutput =>
            {
                Err(SeamError::Refused(Reason::BuildOutput))
            }
            (Some(Flaw::NestedSymlinkIsAccepted), Err(SeamError::Refused(Reason::BuildOutput)))
                if self.world == BuildWorld::SymlinkInDirectory =>
            {
                Ok(vec![Located {
                    shape: Shape::Directory,
                    ..made
                }])
            }
            (Some(Flaw::ListsTheOutputTwice), Ok(mut located))
                if self.world == BuildWorld::Normal =>
            {
                located.push(located[0].clone());
                Ok(located)
            }
            (
                Some(Flaw::NoOutputIsAnEmptyListing),
                Err(SeamError::Refused(Reason::BuildOutput)),
            ) if self.world == BuildWorld::NoOutput => Ok(Vec::new()),
            (
                Some(Flaw::TwoOutputsAnswersTheFirst),
                Err(SeamError::Refused(Reason::BuildOutput)),
            ) if self.world == BuildWorld::TwoOutputs => Ok(vec![made]),
            (Some(Flaw::SymlinkIsAccepted), Err(SeamError::Refused(Reason::BuildOutput)))
                if self.world == BuildWorld::SymlinkInOutput =>
            {
                Ok(vec![made])
            }
            (Some(Flaw::FailureHasNoTail), Err(SeamError::Permanent(_))) => {
                Err(SeamError::Permanent("the build failed".to_owned()))
            }
            (Some(Flaw::FailurePadded), Err(SeamError::Permanent(why))) => {
                let head = match &self.world {
                    BuildWorld::BuildFails { head, .. } => head.as_str(),
                    _ => "",
                };
                Err(SeamError::Permanent(format!("{head}{why}")))
            }
            (Some(Flaw::UnstartableIsPermanent), Err(SeamError::Unknown(why))) => {
                Err(SeamError::Permanent(why))
            }
            (_, result) => result,
        }
    }

    fn test(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<TestOutcome, SeamError> {
        let (system, target) = self.seen(inv, true);
        let run = Invocation {
            system: &system,
            target: &target,
            ..*inv
        };
        let beat = self.tampered(heartbeat);
        let result = self.inner.test(&run, &beat);
        let result = if beat.stopped.get() {
            Err(SeamError::Refused(Reason::LeaseLost))
        } else {
            result
        };
        self.sync(inv);
        let passes = self.in_world(Flaw::NothingRanPasses, &BuildWorld::TestRunsNothing)
            || self.in_world(
                Flaw::UnparseableReportPasses,
                &BuildWorld::ReportUnparseable,
            )
            || (self.is(Flaw::FailureIsAPass)
                && matches!(self.world, BuildWorld::TestFails { .. }));
        match result {
            Err(SeamError::Unknown(why)) if self.is(Flaw::UnstartableTestIsPermanent) => {
                Err(SeamError::Permanent(why))
            }
            result => result.map(|outcome| {
                let counted = self.is(Flaw::SkippedCountedAsRan)
                    && self.world == BuildWorld::TestSkipsEverything;
                let mut outcome = TestOutcome {
                    passed: outcome.passed || passes || counted,
                    ran: if counted { 1 } else { outcome.ran },
                    ..outcome
                };
                if self.is(Flaw::RanInflated) && outcome.ran >= 1 {
                    outcome.ran *= 7;
                }
                if self.is(Flaw::TestTailGarbage) && !outcome.passed {
                    "zzz".clone_into(&mut outcome.tail);
                }
                outcome
            }),
        }
    }

    fn copy_out(&mut self, out: &Located, into: &mut dyn EntryWriter) -> Result<(), SeamError> {
        let mut suffix = Vec::new();
        // The counter shows from the second build on, and the time once some has passed (only
        // `nothing_run_specific_in_the_bytes` moves it, after `same_command_same_bytes` has
        // read the output), so that each stub is wrong in the scenario of its own and in no
        // scenario that builds once, in one moment, and reads the output.
        if self.is(Flaw::CallCounterInTheBytes) && self.builds > 1 {
            suffix = format!("#{}", self.builds).into_bytes();
        }
        let elapsed = self.clock.now_millis() - START_MS;
        if self.is(Flaw::ClockInTheBytes) && elapsed > 0 {
            suffix = format!("@{elapsed}").into_bytes();
        }
        let directory = out.shape == Shape::Directory;
        if self.is(Flaw::CopyOutOfADirectoryCarriesTheClock) && directory && elapsed > 0 {
            suffix = format!("@{elapsed}").into_bytes();
        }
        let replace = match self.flaw {
            Some(Flaw::CopyOutWritesGarbage) => Some(b"garbage".to_vec()),
            Some(Flaw::CopyOutOfADirectoryWritesGarbage) if directory => Some(b"garbage".to_vec()),
            Some(Flaw::CopyOutWritesAnEmptyFile) => Some(Vec::new()),
            _ => None,
        };
        let mut writer = Suffixed {
            into,
            suffix,
            escape: self.is(Flaw::CopyOutEscapes)
                || (self.is(Flaw::CopyOutOfADirectoryEscapes) && out.shape == Shape::Directory),
            replace,
            extra: self.is(Flaw::CopyOutWritesAnExtraFile),
        };
        self.inner.copy_out(out, &mut writer)
    }
}
