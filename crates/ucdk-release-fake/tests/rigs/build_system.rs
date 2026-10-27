// SPDX-License-Identifier: Apache-2.0
//! The build-system rig: the fake build system in each world, and the stubs that break one
//! promise of a build system in one world.

use std::time::Duration;

use ucdk_core::{Clock, ManualClock};
use ucdk_release_fake::FakeBuildSystem;
use ucdk_release_proto::v1::{BuildSystemKind, Reason};
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
}

/// The time on the clock of a rig before anything moves it.
const START_MS: u64 = 1_000_000;

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
            BuildWorld::BuildFails { tail } => {
                fake.fail_at_call(1);
                fake.failure_tail(tail);
            }
            BuildWorld::Unstartable => fake.unstartable(),
            BuildWorld::TestRunsNothing => fake.test_runs_nothing(BUILD_LABEL),
            BuildWorld::TestFails => fake.test_fails(BUILD_LABEL),
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
        }
    }

    fn later(&self) {
        self.clock.advance(Duration::from_secs(86_400));
    }

    /// The fake writes one file, `out`, that names the label, every argument and every
    /// environment pair of the command that built it.
    fn output_of(&self, inv: &Invocation<'_>) -> Vec<(String, Vec<u8>)> {
        let command = FakeBuildSystem::new(self.kind, &self.clock)
            .command(inv)
            .expect("the fake layers the invocation");
        let env: Vec<String> = command
            .env
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let bytes = format!(
            "built {BUILD_LABEL}\n{}\n{}\n",
            command.argv.join("\n"),
            env.join("\n")
        );
        vec![("out".to_owned(), bytes.into_bytes())]
    }
}

/// A heartbeat that never stops.
struct NeverStops;

impl Heartbeat for NeverStops {
    fn beat(&self) -> Result<(), Stop> {
        Ok(())
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

impl Flawed {
    fn is(&self, flaw: Flaw) -> bool {
        self.flaw == Some(flaw)
    }

    fn in_world(&self, flaw: Flaw, world: &BuildWorld) -> bool {
        self.is(flaw) && self.world == *world
    }
}

impl BuildSystem for Flawed {
    fn kind(&self) -> BuildSystemKind {
        self.inner.kind()
    }

    fn command(&self, inv: &Invocation<'_>) -> Result<Command, SeamError> {
        let mut command = self.inner.command(inv)?;
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
                command
                    .argv
                    .push(format!("--build-number={}", inv.stamp.build_number));
            }
            Some(Flaw::OwnedFlagFirst) => {
                if let Some(at) = command.argv.iter().position(|a| a.contains("/scratch/")) {
                    let owned = command.argv.remove(at);
                    command.argv.insert(2, owned);
                }
            }
            _ => {}
        }
        Ok(command)
    }

    fn build(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<Vec<Located>, SeamError> {
        self.builds += 1;
        let swallowed = self.in_world(Flaw::StopIsSwallowed, &BuildWorld::Progress { events: 3 });
        let result = if swallowed {
            self.inner.build(inv, &NeverStops)
        } else {
            self.inner.build(inv, heartbeat)
        };
        let made = Located {
            label: BUILD_LABEL.to_owned(),
            path: "/out/pkg_out/out".to_owned(),
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
        let swallowed = self.in_world(Flaw::StopIsSwallowed, &BuildWorld::Progress { events: 3 });
        let result = if swallowed {
            self.inner.test(inv, &NeverStops)
        } else {
            self.inner.test(inv, heartbeat)
        };
        let passes = self.in_world(Flaw::NothingRanPasses, &BuildWorld::TestRunsNothing)
            || self.in_world(Flaw::FailureIsAPass, &BuildWorld::TestFails)
            || self.in_world(
                Flaw::UnparseableReportPasses,
                &BuildWorld::ReportUnparseable,
            );
        result.map(|outcome| TestOutcome {
            passed: outcome.passed || passes,
            ..outcome
        })
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
        let replace = match self.flaw {
            Some(Flaw::CopyOutWritesGarbage) => Some(b"garbage".to_vec()),
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
