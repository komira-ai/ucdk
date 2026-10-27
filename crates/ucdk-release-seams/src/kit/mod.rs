// SPDX-License-Identifier: Apache-2.0
//! The kits: one list of named scenarios per seam, written against the trait only, that every
//! implementation of the seam must pass unmodified.
//!
//! A kit cannot make an implementation fail by itself, so each takes a rig: the small trait the
//! implementation's author writes to put the implementation into the situation a scenario
//! needs (a tree with a symlink in it, a build that lists two outputs, a registry whose answer
//! was lost). A rig only builds worlds; every assertion is the kit's.
//!
//! A scenario that fails ends with a [`Failure`]; the runner catches it and names the scenario,
//! so a kit reports every scenario that failed, not only the first. The kits read no clock and
//! wait for nothing: time moves only when a scenario asks its rig to move it.
//!
//! A scenario may be gated: it names a thing a rig can lack (a channel that stages, a registry
//! that can burn a number), as a [`Need`], and the runner then records it as
//! [`Verdict::Skipped`] with it. A skip is a legal outcome and never a pass: [`Report::passed`] does not list it,
//! [`Report::skipped`] does, and a rig that skips a scenario says so in its own test.
//!
//! - [`run_build_system_kit`], [`run_grant_source_kit`], [`run_output_files_kit`],
//!   [`run_process_runner_kit`], [`run_publisher_kit`] and [`run_store_kit`] run a whole kit and
//!   return its [`Report`].
//! - [`build_system_scenarios`], [`grant_source_scenarios`], [`output_files_scenarios`],
//!   [`process_runner_scenarios`], [`publisher_scenarios`] and [`store_scenarios`] give the
//!   scenarios one by one, for a test per scenario.

use std::any::Any;
use std::fmt::Debug;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, Ordering};

use ucdk_release_proto::v1::{ArtifactType, Coordinate};

use crate::{EntryWriter, Heartbeat, SeamError, Stop, valid_rel};

mod build_system;
mod grant_bounds;
mod grant_claims;
mod grant_exceptional;
mod grant_secret;
mod grant_source;
mod output_files;
mod process_runner;
mod publisher;
mod publisher_access;
mod publisher_gates;
mod publisher_grants;
mod publisher_list;
mod publisher_more;
mod publisher_private;
mod publisher_reads;
mod publisher_rig;
mod publisher_shapes;
mod publisher_visible;
mod store;

pub use build_system::{
    BUILD_LABEL, BuildSystemRig, BuildWorld, build_system_scenarios, run_build_system_kit,
};
pub use grant_source::{
    COMMIT_SCOPE, GrantContent, GrantSetup, GrantSourceRig, IdentityTokens, JobClaims, LOCATION,
    READ_SCOPE, STAGE_SCOPE, exceptional_job, federated_repository, grant_source_scenarios,
    matching_job, run_grant_source_kit,
};
pub use output_files::{Node, OutputFilesRig, output_files_scenarios, run_output_files_kit};
pub use process_runner::{Behavior, ProcessRig, process_runner_scenarios, run_process_runner_kit};
pub use publisher::run_publisher_kit;
pub use publisher_gates::skips_that_name_a_declared_capability;
pub use publisher_list::publisher_scenarios;
pub use publisher_rig::{
    Arrangement, ChannelShape, Payload, Placement, PublisherRig, ReadStep, Sibling,
    every_field_in_a_set_of_its_own,
};
pub use store::{StoreRig, run_store_kit, store_scenarios};

/// A coordinate of `name`, with the channel, type, version and platform every kit uses.
pub fn coordinate(name: &str) -> Coordinate {
    Coordinate {
        channel: "stable".to_owned(),
        r#type: ArtifactType::File as i32,
        name: name.to_owned(),
        version: "1.2.3".to_owned(),
        platform: "linux-x86_64".to_owned(),
    }
}

/// How a scenario ends when it fails: the text says what was expected and what was seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure(pub String);

/// Ends the scenario that is running.
fn fail(why: String) -> ! {
    panic::resume_unwind(Box::new(Failure(why)))
}

// Both sides are taken by value so that a call reads `same(got, want, ..)` over temporaries.
#[allow(clippy::needless_pass_by_value)]
fn same<T: PartialEq + Debug>(got: T, want: T, what: &str) {
    if got != want {
        fail(format!("{what}: got {got:?}, want {want:?}"));
    }
}

/// The value of a call the scenario needs to have succeeded.
fn must<T, E: Debug>(result: Result<T, E>, what: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => fail(format!("{what} failed: {error:?}")),
    }
}

/// The error of a call the scenario needs to have failed.
fn must_err<T>(result: Result<T, SeamError>, what: &str) -> SeamError {
    match result {
        Ok(_) => fail(format!("{what} succeeded")),
        Err(error) => error,
    }
}

/// The value a scenario needs to be there.
fn must_some<T>(value: Option<T>, what: &str) -> T {
    match value {
        Some(value) => value,
        None => fail(format!("{what} is missing")),
    }
}

/// Whether `error` is a `Permanent` one that names `path`. Both halves are always asked, so
/// that no scenario holds a branch of its own for the answer.
fn permanent_naming(error: &SeamError, path: &str) -> bool {
    matches!(error, SeamError::Permanent(_)) & error.to_string().contains(path)
}

/// What a scenario needs of a rig and the rig can lack. A gate answers the need the rig does not
/// meet, so that a skip can be held to the rig's own declarations: a skip that names a capability
/// the rig declares is a defect of the gate, and the declarations scenario of the publisher kit
/// fails it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// A channel of any shape.
    Channel,
    /// A channel whose reads need no grant.
    PublicChannel,
    /// A channel whose every read needs a grant.
    PrivateChannel,
    /// A channel that stages and addresses staged state by group.
    GroupedChannel,
    /// A channel that stages, whatever else it is.
    StagingChannel,
    /// A channel whose commit carries the bytes, whatever else it is.
    UnstagedChannel,
    /// A channel that commits a set at once.
    SetCommit,
    /// A channel that does not stage and whose commit does not replace what is there.
    ConditionalCommit,
    /// A coordinate the channel cannot hold, named by the rig.
    UnrepresentableCoordinate,
    /// A registry that seals a version by the group that first wrote it.
    SealingRegistry,
    /// A registry that can burn a number.
    BurningRegistry,
}

impl Need {
    /// Why a rig that lacks the need cannot run the scenario.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Channel => "the rig declares no channel",
            Self::PublicChannel => "the rig declares no public channel",
            Self::PrivateChannel => "the rig declares no private channel",
            Self::GroupedChannel => {
                "the rig declares no channel that stages and is addressed by group"
            }
            Self::StagingChannel => "the rig declares no channel that stages",
            Self::UnstagedChannel => "the rig declares no channel that does not stage",
            Self::SetCommit => "the rig declares no channel that commits a set at once",
            Self::ConditionalCommit => {
                "the rig declares no channel that does not stage and is a conditional put"
            }
            Self::UnrepresentableCoordinate => {
                "the rig names no coordinate its channel cannot hold"
            }
            Self::SealingRegistry => "the registry does not seal a version by its first group",
            Self::BurningRegistry => "the registry cannot burn a number",
        }
    }
}

/// One named scenario of a kit over the rig type `R`.
pub struct Scenario<R> {
    pub name: &'static str,
    run: fn(&R),
    gate: Option<fn(&R) -> Option<Need>>,
}

impl<R> Scenario<R> {
    pub fn new(name: &'static str, run: fn(&R)) -> Self {
        Self {
            name,
            run,
            gate: None,
        }
    }

    /// The same scenario, run only where `gate` answers `None`; an answer is the [`Need`] the rig
    /// does not meet, and the runner records the scenario as skipped with it.
    #[must_use]
    pub fn gated(mut self, gate: fn(&R) -> Option<Need>) -> Self {
        self.gate = Some(gate);
        self
    }

    /// What the scenario needs that `rig` lacks, which is why it does not run; `None` when it does.
    pub fn skip_need(&self, rig: &R) -> Option<Need> {
        self.gate.and_then(|gate| gate(rig))
    }

    /// Runs the scenario once, whatever its gate says; `Err` says what failed.
    pub fn run_on(&self, rig: &R) -> Result<(), String> {
        panic::catch_unwind(AssertUnwindSafe(|| (self.run)(rig)))
            .map_err(|payload| describe(&*payload))
    }

    /// Runs the scenario unless its gate skips it.
    pub fn verdict_on(&self, rig: &R) -> Verdict {
        // A gate may build a channel, and what a rig builds may panic: that is a failed
        // scenario, as one that panics in its body is.
        let gate = panic::catch_unwind(AssertUnwindSafe(|| self.skip_need(rig)));
        match gate {
            Err(payload) => Verdict::Failed(describe(&*payload)),
            Ok(Some(need)) => Verdict::Skipped(need),
            Ok(None) => match self.run_on(rig) {
                Ok(()) => Verdict::Passed,
                Err(why) => Verdict::Failed(why),
            },
        }
    }
}

fn describe(payload: &(dyn Any + Send)) -> String {
    payload.downcast_ref::<Failure>().map_or_else(
        || "the scenario panicked".to_owned(),
        |failure| failure.0.clone(),
    )
}

/// How one scenario ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Passed,
    /// The scenario failed, for this reason.
    Failed(String),
    /// The rig lacks what the scenario needs, and the scenario did not run. A skip is not a pass.
    Skipped(Need),
}

/// What one scenario came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub name: &'static str,
    pub verdict: Verdict,
}

/// What a kit found: one outcome per scenario, in the kit's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub outcomes: Vec<Outcome>,
}

impl Report {
    /// The names of the scenarios that failed.
    pub fn failed(&self) -> Vec<&'static str> {
        self.named(|v| matches!(v, Verdict::Failed(_)))
    }

    /// The names of the scenarios that ran and passed: a skipped scenario is not among them.
    pub fn passed(&self) -> Vec<&'static str> {
        self.named(|v| matches!(v, Verdict::Passed))
    }

    /// The names of the scenarios the rig skipped, in order.
    pub fn skipped(&self) -> Vec<&'static str> {
        self.named(|v| matches!(v, Verdict::Skipped(_)))
    }

    /// Why each skipped scenario did not run, as `name: reason`.
    pub fn skip_reasons(&self) -> Vec<String> {
        self.outcomes
            .iter()
            .filter_map(|o| match o.verdict {
                Verdict::Skipped(need) => Some(format!("{}: {}", o.name, need.reason())),
                _ => None,
            })
            .collect()
    }

    fn named(&self, keep: impl Fn(&Verdict) -> bool) -> Vec<&'static str> {
        self.outcomes
            .iter()
            .filter(|o| keep(&o.verdict))
            .map(|o| o.name)
            .collect()
    }

    /// The names of every scenario, in order.
    pub fn names(&self) -> Vec<&'static str> {
        self.outcomes.iter().map(|o| o.name).collect()
    }

    /// Panics, listing every failed scenario with its reason, unless none failed. A skipped
    /// scenario does not fail the kit; a test that must know of it reads [`Report::skipped`].
    pub fn assert_conforms(&self) {
        let lines: Vec<String> = self
            .outcomes
            .iter()
            .filter_map(|o| match &o.verdict {
                Verdict::Failed(why) => Some(format!("{}: {why}", o.name)),
                _ => None,
            })
            .collect();
        assert!(lines.is_empty(), "the kit failed:\n{}", lines.join("\n"));
    }
}

/// Runs every scenario against `rig`.
pub fn run_scenarios<R>(rig: &R, scenarios: &[Scenario<R>]) -> Report {
    Report {
        outcomes: scenarios
            .iter()
            .map(|s| Outcome {
                name: s.name,
                verdict: s.verdict_on(rig),
            })
            .collect(),
    }
}

/// A heartbeat that counts its beats, answers `Stop` from the `stop_at`th on (0: never), and
/// tells a watcher about each beat.
pub struct Beats<'a> {
    count: AtomicU32,
    stop_at: u32,
    watcher: Option<&'a dyn Fn(u32)>,
}

impl<'a> Beats<'a> {
    /// A heartbeat that never stops.
    pub fn counting() -> Self {
        Self {
            count: AtomicU32::new(0),
            stop_at: 0,
            watcher: None,
        }
    }

    /// A heartbeat that answers `Stop` from its `n`th beat on.
    pub fn stopping_at(n: u32) -> Self {
        Self {
            stop_at: n,
            ..Self::counting()
        }
    }

    /// The same heartbeat, telling `watcher` the number of each beat as it comes.
    #[must_use]
    pub fn watched_by(mut self, watcher: &'a dyn Fn(u32)) -> Self {
        self.watcher = Some(watcher);
        self
    }

    /// The beats so far.
    pub fn count(&self) -> u32 {
        self.count.load(Ordering::SeqCst)
    }
}

impl Heartbeat for Beats<'_> {
    fn beat(&self) -> Result<(), Stop> {
        let n = self.count.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(watcher) = self.watcher {
            watcher(n);
        }
        if self.stop_at != 0 && n >= self.stop_at {
            Err(Stop)
        } else {
            Ok(())
        }
    }
}

/// An entry writer that keeps every call it was given, valid or not, and refuses a path that is
/// not a relative path of an entry, as a store does.
#[derive(Debug, Default)]
pub(crate) struct Recorder {
    pub(crate) calls: Vec<(String, Vec<u8>)>,
}

impl EntryWriter for Recorder {
    fn put_file(&mut self, rel: &str, bytes: &[u8]) -> Result<(), SeamError> {
        self.calls.push((rel.to_owned(), bytes.to_vec()));
        if valid_rel(rel) {
            Ok(())
        } else {
            Err(SeamError::Refused(
                ucdk_release_proto::v1::Reason::SetMember,
            ))
        }
    }
}
