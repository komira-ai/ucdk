// SPDX-License-Identifier: Apache-2.0
//! The engine: a request names a stage and what to run in it.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use ucdk_ledger::{DEFAULT_LEASE_TTL, Ledger};
use ucdk_release::machine::Checked;
use ucdk_release::selection::Selection;
use ucdk_release_proto::v1::{BuildStamp, Reason, Verdict};
use ucdk_release_seams::{ArtifactStore, BuildSystem, Pause, Publisher};

use crate::outcome::Outcome;

/// What the driver gives a run: who it is, and the facts every action of it sees.
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    pub id: &'a str,
    pub attempt: u32,
    pub revision: &'a str,
    pub stamp: &'a BuildStamp,
}

/// The seams the engine works through.
pub struct Seams<'a> {
    pub build_system: &'a mut dyn BuildSystem,
    pub store: &'a mut dyn ArtifactStore,
    pub channels: BTreeMap<String, &'a mut dyn Publisher>,
    pub pause: &'a mut dyn Pause,
}

/// What to run: a stage, and the `--only` selectors, none meaning all of the stage.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub stage: String,
    pub only: Selection,
}

/// The engine of one run.
pub struct Engine<'a> {
    machine: &'a Checked,
    ledger: &'a dyn Ledger,
    run: Run<'a>,
    seams: Seams<'a>,
    scratch: &'a Path,
    ttl: Duration,
}

impl<'a> Engine<'a> {
    pub fn new(
        machine: &'a Checked,
        ledger: &'a dyn Ledger,
        run: Run<'a>,
        seams: Seams<'a>,
        scratch: &'a Path,
    ) -> Self {
        Self {
            machine,
            ledger,
            run,
            seams,
            scratch,
            ttl: DEFAULT_LEASE_TTL,
        }
    }

    #[must_use]
    pub fn with_lease_ttl(mut self, ttl: Duration) -> Self {
        self.ttl = ttl;
        self
    }

    /// Not implemented yet: every request fails.
    pub fn handle(&mut self, request: &Request) -> Outcome {
        let _ = (
            self.machine,
            self.ledger,
            &self.run,
            &self.seams,
            self.scratch,
            self.ttl,
            request,
        );
        Outcome::halted(Verdict::Failed, Reason::Unspecified, "not implemented")
    }
}
