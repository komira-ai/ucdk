// SPDX-License-Identifier: Apache-2.0
//! The seam: what is asked of the engine and what comes back, as plain Rust values.

use std::fmt;

use ucdk_plan::{Conflict, Lowered, Plan, PlanHash, Retained};
use ucdk_provider::NodePath;

use crate::error::{EngineError, short};

/// What a plan is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Make the world match the file.
    Apply,
    /// Remove everything the file names (the desired state is none of it).
    Destroy,
}

#[derive(Debug, Clone)]
pub enum Request {
    /// Observe and plan; changes nothing.
    Plan { desired: Lowered, mode: Mode },
    /// Carry out the approved `plan`. `plan_hash` is the hash the approver pinned: the run is
    /// refused if `plan` does not carry it, or if the plan made from what is observed now
    /// does not. With `rollback`, a failed run deletes what it created.
    Apply {
        desired: Lowered,
        plan: Plan,
        plan_hash: PlanHash,
        rollback: bool,
    },
    /// Carry out the approved destroy plan, under the same hash rule as `Apply`.
    Destroy {
        desired: Lowered,
        plan: Plan,
        plan_hash: PlanHash,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The plan of a `Request::Plan`.
    Planned(Plan),
    /// The run finished.
    Done(Report),
    /// The run was refused before anything was changed.
    Refused(Refusal),
    /// The run stopped; `Failure::report` says what had been done.
    Failed(Failure),
}

/// Why a run was refused whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// An object is not ours, or the ledger and the cloud disagree: every offender is listed.
    Conflicts(Vec<Conflict>),
    /// A destroy of a file that names nothing.
    EmptyScope,
    /// The plan handed in does not carry the pinned hash.
    PlanHashMismatch { approved: PlanHash, plan: PlanHash },
    /// The plan made from the world as it is now is not the approved one.
    PlanChanged { approved: PlanHash, now: PlanHash },
    /// The file holds nodes of a kind that carries no stamp of its own, which the engine does
    /// not apply yet.
    UnsupportedStampMode(Vec<NodePath>),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflicts(list) => {
                let lines: Vec<String> = list.iter().map(ToString::to_string).collect();
                write!(f, "refused: {}", lines.join("; "))
            }
            Self::EmptyScope => f.write_str("refused: the file names nothing to destroy"),
            Self::PlanHashMismatch { approved, plan } => write!(
                f,
                "refused: the plan is {}, the approved hash is {}",
                short(plan),
                short(approved)
            ),
            Self::PlanChanged { approved, now } => write!(
                f,
                "refused: the plan is now {}, the approved hash is {}",
                short(now),
                short(approved)
            ),
            Self::UnsupportedStampMode(paths) => {
                let list: Vec<String> = paths.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "refused: kinds that carry no stamp of their own are not applied yet: {}",
                    list.join(", ")
                )
            }
        }
    }
}

/// What a run did, by node, in the order it did it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Objects this run created.
    pub created: Vec<NodePath>,
    /// Objects this run converged in place.
    pub updated: Vec<NodePath>,
    /// Objects an earlier, interrupted run had begun: found in place and confirmed, with no
    /// create.
    pub resumed: Vec<NodePath>,
    /// Objects this run deleted.
    pub deleted: Vec<NodePath>,
    /// Records dropped because the object was already gone.
    pub reaped: Vec<NodePath>,
    /// Objects left alone on purpose, and why.
    pub retained: Vec<(NodePath, Retained)>,
    /// Objects the provider reports as still converging: nothing was written to them and their
    /// records are not confirmed. The run still succeeds; a later run settles them.
    pub converging: Vec<NodePath>,
    /// Objects a rollback deleted (or found gone).
    pub rolled_back: Vec<NodePath>,
    /// Objects a rollback could not delete.
    pub rollback_failed: Vec<(NodePath, EngineError)>,
}

/// A run that stopped at a step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// What had been done before it stopped, and what a rollback did.
    pub report: Report,
    /// The node of the step that failed; none when nothing was started (the world could not
    /// be read).
    pub at: Option<NodePath>,
    pub error: EngineError,
}
