// SPDX-License-Identifier: Apache-2.0
//! The action input digest: the `input` of an `action` record. On a resume it decides whether a
//! `Confirmed` action is the same work (`NOOP`) or its inputs changed. It is SHA-256 over the
//! domain tag `ucdk/release-action-input/1`, a text naming the kind of the action, and the
//! fields of that kind, fed as the hashes of [`crate::hashes`] are.
//!
//! Not in any of them: the validations of the action (a validation has its own record and its
//! own subject), the bounds `Publish.attempts`, `Publish.polls` and `Validation.wait_polls` (a
//! bound changes how long a run tries, not what it does), the action's name (it is in the
//! record's name), the timestamp, the run id and attempt, the report path of a test, and
//! everything of other actions. Flags and values are the authored text, placeholders
//! unsubstituted.

use std::fmt;

use ucdk_core::{Framed, Hash256};
use ucdk_release_proto::v1::action::Kind as ActionKind;
use ucdk_release_proto::v1::{BuildStamp, Stage};

use crate::hashes::{build_inputs_hash, number, pairs, stamped, strings, system, text};
use crate::machine::{Checked, stage_platform};

const ACTION_INPUT_DOMAIN: &[u8] = b"ucdk/release-action-input/1\n";

/// What a run gives an action besides the machine.
#[derive(Debug, Clone, Copy)]
pub struct Given<'a> {
    pub revision: &'a str,
    /// The build stamp of the run; its timestamp is not hashed.
    pub stamp: &'a BuildStamp,
    /// The sealed set a Publish or Smoke action consumes; no other action is given one.
    pub set: Option<&'a Hash256>,
}

/// Why an action input digest could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputError {
    UnknownStage(String),
    UnknownAction {
        stage: String,
        action: String,
    },
    /// A Publish or Smoke action was given no set.
    SetRequired(String),
    /// A Build or Test action was given a set, which it does not consume.
    SetNotAllowed(String),
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputError::UnknownStage(stage) => write!(f, "the machine has no stage {stage}"),
            InputError::UnknownAction { stage, action } => {
                write!(f, "the stage {stage} has no action {action}")
            }
            InputError::SetRequired(action) => {
                write!(
                    f,
                    "the action {action} consumes a sealed set, and none was given"
                )
            }
            InputError::SetNotAllowed(action) => {
                write!(f, "the action {action} consumes no set, and one was given")
            }
        }
    }
}

/// The input digest of the action `action` of the stage `stage`.
pub fn action_input(
    machine: &Checked,
    stage: &str,
    action: &str,
    given: &Given<'_>,
) -> Result<Hash256, InputError> {
    let found = machine
        .stage(stage)
        .ok_or_else(|| InputError::UnknownStage(stage.to_owned()))?;
    // A checked machine has a kind on every action, so an action with none is the same as one
    // that is not there.
    let kind = found
        .actions
        .iter()
        .filter(|a| a.name == action)
        .find_map(|a| a.kind.as_ref())
        .ok_or_else(|| InputError::UnknownAction {
            stage: stage.to_owned(),
            action: action.to_owned(),
        })?;
    let hash = Framed::new(ACTION_INPUT_DOMAIN);
    match (kind, given.set) {
        (ActionKind::Build(_) | ActionKind::Test(_), Some(_)) => {
            Err(InputError::SetNotAllowed(action.to_owned()))
        }
        (ActionKind::Publish(_) | ActionKind::Smoke(_), None) => {
            Err(InputError::SetRequired(action.to_owned()))
        }
        (ActionKind::Build(build), None) => {
            let inputs = build_inputs_hash(machine, build, given.revision, given.stamp);
            Ok(text(text(hash, "build"), inputs.as_str()).finish())
        }
        (ActionKind::Test(test), None) => {
            Ok(test_fields(machine, found, text(hash, "test"), given, &test.targets).finish())
        }
        (ActionKind::Publish(p), Some(set)) => Ok(consumed(
            text(hash, "publish"),
            set,
            given.revision,
            (&p.platform, &p.channel, &p.artifacts),
        )
        .finish()),
        (ActionKind::Smoke(s), Some(set)) => Ok(consumed(
            text(hash, "smoke"),
            set,
            given.revision,
            (&s.platform, &s.channel, &s.artifacts),
        )
        .finish()),
    }
}

/// The fields of a Test action after its kind: where and what it runs, and how, as authored.
fn test_fields(
    machine: &Checked,
    stage: &Stage,
    hash: Framed,
    given: &Given<'_>,
    targets: &[ucdk_release_proto::v1::Target],
) -> Framed {
    let hash = text(hash, given.revision);
    let hash = text(hash, stage_platform(stage).unwrap_or_default());
    let hash = stamped(hash, given.stamp);
    let build_system = machine.machine().build_system.clone().unwrap_or_default();
    let hash = system(hash, &build_system);
    targets.iter().fold(number(hash, targets.len()), |h, t| {
        pairs(strings(text(h, &t.label), &t.flags), &t.env)
    })
}

/// The fields of a Publish or Smoke action after its kind: the set, the revision, the platform,
/// the channel and the artifacts in the order listed.
fn consumed(
    hash: Framed,
    set: &Hash256,
    revision: &str,
    (platform, channel, artifacts): (&String, &String, &Vec<String>),
) -> Framed {
    let hash = text(text(text(hash, set.as_str()), revision), platform);
    strings(text(hash, channel), artifacts)
}
