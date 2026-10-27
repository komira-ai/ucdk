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
//! - [`run_build_system_kit`], [`run_grant_source_kit`], [`run_output_files_kit`],
//!   [`run_process_runner_kit`] and [`run_store_kit`] run a whole kit and return its [`Report`];
//!   the kit of the other seam joins them in the part that follows.
//! - [`build_system_scenarios`], [`grant_source_scenarios`], [`output_files_scenarios`],
//!   [`process_runner_scenarios`] and [`store_scenarios`] give the scenarios one by one, for a
//!   test per scenario.

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

/// One named scenario of a kit over the rig type `R`.
pub struct Scenario<R> {
    pub name: &'static str,
    run: fn(&R),
}

impl<R> Scenario<R> {
    pub fn new(name: &'static str, run: fn(&R)) -> Self {
        Self { name, run }
    }

    /// Runs the scenario once; `Err` says what failed.
    pub fn run_on(&self, rig: &R) -> Result<(), String> {
        panic::catch_unwind(AssertUnwindSafe(|| (self.run)(rig)))
            .map_err(|payload| describe(&*payload))
    }
}

fn describe(payload: &(dyn Any + Send)) -> String {
    payload.downcast_ref::<Failure>().map_or_else(
        || "the scenario panicked".to_owned(),
        |failure| failure.0.clone(),
    )
}

/// What one scenario came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub name: &'static str,
    pub result: Result<(), String>,
}

/// What a kit found: one outcome per scenario, in the kit's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub outcomes: Vec<Outcome>,
}

impl Report {
    /// The names of the scenarios that failed.
    pub fn failed(&self) -> Vec<&'static str> {
        self.outcomes
            .iter()
            .filter(|o| o.result.is_err())
            .map(|o| o.name)
            .collect()
    }

    /// The names of every scenario, in order.
    pub fn names(&self) -> Vec<&'static str> {
        self.outcomes.iter().map(|o| o.name).collect()
    }

    /// Panics, listing every failed scenario with its reason, unless all passed.
    pub fn assert_conforms(&self) {
        let lines: Vec<String> = self
            .outcomes
            .iter()
            .filter_map(|o| {
                o.result
                    .as_ref()
                    .err()
                    .map(|why| format!("{}: {why}", o.name))
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
                result: s.run_on(rig),
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
