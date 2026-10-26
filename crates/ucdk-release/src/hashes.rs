// SPDX-License-Identifier: Apache-2.0
//! The hashes that approve things: the set hash (what a build produced), the machine hash,
//! the run hash (what an approver agreed to run) and the build-inputs hash (what a stored set
//! was built from). Each is SHA-256 over its own domain tag and length-framed fields, so two
//! lists that concatenate to the same bytes still hash differently. The action input digest,
//! which says whether a confirmed action is the same work on a resume, is in
//! [`crate::action_input`] and is fed the same way.
//!
//! A number is fed as its decimal text, an enum as its number in decimal, a bool as `1` or
//! `0`, a list as its length in decimal and then its items, and a map as its length and then
//! each key and value in sorted key order. Nothing a hash covers is observed state, and no
//! credential or token enters any of them.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use prost::Message;
use ucdk_core::{Framed, Hash256, order};
use ucdk_release_proto::v1::action::Kind as ActionKind;
use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{
    ArtifactDecl, Build, BuildStamp, BuildSystem, Machine, Manifest, ReleaseSet, Stage,
};

use crate::machine::{Checked, stage_platform};
use crate::selection::Selection;

const SET_DOMAIN: &[u8] = b"ucdk/release-set/1\n";
const MACHINE_DOMAIN: &[u8] = b"ucdk/release-machine/1\n";
const RUN_DOMAIN: &[u8] = b"ucdk/release-run/1\n";
const BUILD_INPUTS_DOMAIN: &[u8] = b"ucdk/release-build-inputs/1\n";

/// A text field: the UTF-8 bytes of the string.
pub(crate) fn text(hash: Framed, value: &str) -> Framed {
    hash.field(value.as_bytes())
}

/// A number field: the decimal text of the number. An enum is its number, a bool `1` or `0`.
pub(crate) fn number(hash: Framed, value: impl fmt::Display) -> Framed {
    hash.field(value.to_string().as_bytes())
}

/// A string list: a count, then one text field per item.
pub(crate) fn strings(hash: Framed, values: &[String]) -> Framed {
    values
        .iter()
        .fold(number(hash, values.len()), |h, v| text(h, v))
}

/// A map of text to text: a count, then each key and value in sorted key order.
pub(crate) fn pairs(hash: Framed, values: &BTreeMap<String, String>) -> Framed {
    values
        .iter()
        .fold(number(hash, values.len()), |h, (k, v)| text(text(h, k), v))
}

/// The version, build number and source commit of a stamp: the part of a build stamp that
/// every hash covers. The timestamp is pinned by the run record and is never hashed.
pub(crate) fn stamped(hash: Framed, stamp: &BuildStamp) -> Framed {
    let hash = text(hash, &stamp.version);
    text(number(hash, stamp.build_number), &stamp.source_commit)
}

/// The build system as authored: kind, binary, config file, mode, base flags, base environment.
pub(crate) fn system(hash: Framed, system: &BuildSystem) -> Framed {
    let hash = text(number(hash, system.kind), &system.binary);
    let hash = text(text(hash, &system.config_file), &system.mode);
    pairs(strings(hash, &system.flags), &system.env)
}

/// The hash of a set: revision, platform, the producer (stage, then environment) and, for each
/// member sorted bytewise by name, every field of its manifest and its metadata arm. The run
/// and attempt that built the set (`produced_by`), the set's own `set_hash` field and the
/// stamp's timestamp are not covered, and nor is anything about how the members were built.
pub fn set_hash(set: &ReleaseSet) -> Hash256 {
    let producer = set.producer.clone().unwrap_or_default();
    let mut hash = Framed::new(SET_DOMAIN);
    for value in [
        &set.revision,
        &set.platform,
        &producer.stage,
        &producer.environment,
    ] {
        hash = text(hash, value);
    }
    let empty = Manifest::default();
    let mut members: Vec<&Manifest> = set
        .members
        .iter()
        .map(|m| m.manifest.as_ref().unwrap_or(&empty))
        .collect();
    members.sort_by(|a, b| a.name.cmp(&b.name));
    hash = number(hash, members.len());
    for m in members {
        let stamp = m.stamp.clone().unwrap_or_default();
        hash = text(hash, &m.name);
        hash = text(hash, &m.version);
        hash = number(hash, m.r#type);
        hash = text(hash, &m.platform);
        hash = text(hash, &m.file);
        hash = number(hash, m.size);
        hash = text(hash, &m.sha256);
        hash = number(hash, stamp.build_number);
        hash = text(hash, &stamp.source_commit);
        hash = match &m.metadata {
            None => number(hash, 0),
            Some(Metadata::FileMeta(meta)) => pairs(number(hash, 10), &meta.labels),
            Some(Metadata::Oci(oci)) => text(text(number(hash, 11), &oci.os), &oci.cpu),
        };
    }
    hash.finish()
}

/// The hash of a machine: its canonical encoding (fields in number order, map entries in key
/// order, defaults omitted; the generated maps are ordered), fed as one field.
pub fn machine_hash(machine: &Machine) -> Hash256 {
    Framed::new(MACHINE_DOMAIN)
        .field(&machine.encode_to_vec())
        .finish()
}

/// Why a run hash could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunHashError {
    UnknownStage(String),
    /// The stage holds a Publish or Smoke action and consumes a sealed set, and none was given.
    SetRequired,
    /// The stage holds no Publish or Smoke action, so it consumes no set, and one was given.
    SetNotAllowed,
}

impl fmt::Display for RunHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunHashError::UnknownStage(name) => write!(f, "the machine has no stage {name}"),
            RunHashError::SetRequired => f.write_str(
                "a stage with a Publish or Smoke action is approved with the set it consumes",
            ),
            RunHashError::SetNotAllowed => f.write_str(
                "a stage with no Publish or Smoke action consumes no set, so none is approved with it",
            ),
        }
    }
}

/// Whether `stage` holds a Publish or Smoke action, which is what makes it consume the sealed
/// set of the Build stage it needs.
fn consumes_a_set(stage: &Stage) -> bool {
    stage
        .actions
        .iter()
        .any(|a| matches!(a.kind, Some(ActionKind::Publish(_) | ActionKind::Smoke(_))))
}

/// The hash of a run: the machine, the stage, the selection, the revision, the stage's
/// platform, the build stamp (version, build number and source commit; the timestamp is not
/// covered) and, only for a stage that holds a Publish or Smoke action, the sealed set it
/// consumes. It covers no observed state, no run id, no attempt and no ledger.
pub fn run_hash(
    machine: &Checked,
    stage: &str,
    selection: &Selection,
    revision: &str,
    stamp: &BuildStamp,
    sealed: Option<&Hash256>,
) -> Result<Hash256, RunHashError> {
    let found = machine
        .stage(stage)
        .ok_or_else(|| RunHashError::UnknownStage(stage.to_owned()))?;
    let set = match (consumes_a_set(found), sealed) {
        (true, None) => return Err(RunHashError::SetRequired),
        (false, Some(_)) => return Err(RunHashError::SetNotAllowed),
        (_, set) => set,
    };
    let mut hash = text(
        Framed::new(RUN_DOMAIN),
        machine_hash(machine.machine()).as_str(),
    );
    hash = text(hash, stage);
    hash = strings(hash, &selection.texts());
    hash = text(hash, revision);
    hash = text(hash, stage_platform(found).unwrap_or_default());
    hash = stamped(hash, stamp);
    if let Some(set) = set {
        hash = text(hash, set.as_str());
    }
    Ok(hash.finish())
}

/// The hash of what a Build action builds from: its platform, the revision, the build stamp
/// without its timestamp, the build system as authored and, for each artifact the action lists
/// (the list is closed under `needs`) in dependency order, its declaration with its target as
/// authored. Editing another stage, a channel, a validation or a Test action does not change
/// it; editing the build system, a target or an artifact does. Placeholders are hashed as
/// written, never substituted.
pub fn build_inputs_hash(
    machine: &Checked,
    build: &Build,
    revision: &str,
    stamp: &BuildStamp,
) -> Hash256 {
    let mut hash = text(Framed::new(BUILD_INPUTS_DOMAIN), &build.platform);
    hash = text(hash, revision);
    hash = stamped(hash, stamp);
    hash = system(
        hash,
        &machine.machine().build_system.clone().unwrap_or_default(),
    );
    let declarations = declarations(machine, &build.artifacts);
    hash = number(hash, declarations.len());
    for decl in declarations {
        let target = decl.target.clone().unwrap_or_default();
        hash = text(hash, &decl.name);
        hash = number(hash, decl.r#type);
        hash = text(hash, &target.label);
        hash = pairs(strings(hash, &target.flags), &target.env);
        hash = strings(hash, &decl.needs);
        hash = pairs(hash, &decl.labels);
    }
    hash.finish()
}

/// The declarations of `names`, each once, in dependency order: of those ready at once, the one
/// with the smallest name first. A name that is not declared is left out (the reader refuses
/// a machine that has one, so this is only ever asked of a checked machine).
fn declarations<'a>(machine: &'a Checked, names: &[String]) -> Vec<&'a ArtifactDecl> {
    let known: BTreeMap<&str, &ArtifactDecl> = names
        .iter()
        .filter_map(|name| machine.artifact(name))
        .map(|decl| (decl.name.as_str(), decl))
        .collect();
    let deps: BTreeMap<&str, BTreeSet<&str>> = known
        .iter()
        .map(|(name, decl)| (*name, decl.needs.iter().map(String::as_str).collect()))
        .collect();
    let ordered = order(&deps).unwrap_or_else(|rest| rest);
    ordered
        .iter()
        .filter_map(|name| known.get(name).copied())
        .collect()
}
