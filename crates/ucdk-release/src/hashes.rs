// SPDX-License-Identifier: Apache-2.0
//! The hashes that approve things: the set hash (what a build produced), the machine hash,
//! the run hash (what an approver agreed to run) and the build-inputs hash (what a stored set
//! was built from). Each is SHA-256 over its own domain tag and length-framed fields, so two
//! lists that concatenate to the same bytes still hash differently.
//!
//! A number is fed as its decimal text, an enum as its number in decimal, and a list as its
//! length in decimal and then its items. Nothing a hash covers is observed state.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use prost::Message;
use ucdk_core::{Framed, Hash256, order};
use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::{ArtifactDecl, Build, BuildStamp, Machine, Manifest, ReleaseSet};

use crate::machine::{Checked, builds, stage_platform};
use crate::selection::Selection;

const SET_DOMAIN: &[u8] = b"ucdk/release-set/1\n";
const MACHINE_DOMAIN: &[u8] = b"ucdk/release-machine/1\n";
const RUN_DOMAIN: &[u8] = b"ucdk/release-run/1\n";
const BUILD_INPUTS_DOMAIN: &[u8] = b"ucdk/release-build-inputs/1\n";

fn text(hash: Framed, value: &str) -> Framed {
    hash.field(value.as_bytes())
}

fn number(hash: Framed, value: impl fmt::Display) -> Framed {
    hash.field(value.to_string().as_bytes())
}

fn strings(hash: Framed, values: &[String]) -> Framed {
    values
        .iter()
        .fold(number(hash, values.len()), |h, v| text(h, v))
}

/// The hash of a set: revision, platform, the producer (stage, then environment) and, for each
/// member sorted bytewise by name, every field of its manifest and its metadata arm. The run
/// and attempt that built the set (`produced_by`), the set's own `set_hash` field and the
/// stamp's timestamp are not covered.
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
            Some(Metadata::FileMeta(meta)) => {
                let hash = number(number(hash, 10), meta.labels.len());
                meta.labels
                    .iter()
                    .fold(hash, |h, (key, value)| text(text(h, key), value))
            }
            Some(Metadata::Oci(oci)) => text(text(number(hash, 11), &oci.os), &oci.cpu),
        };
    }
    hash.finish()
}

/// The hash of a machine: its canonical encoding (fields in number order, map entries in key
/// order, defaults omitted), fed as one field.
pub fn machine_hash(machine: &Machine) -> Hash256 {
    Framed::new(MACHINE_DOMAIN)
        .field(&machine.encode_to_vec())
        .finish()
}

/// Why a run hash could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunHashError {
    UnknownStage(String),
    /// The stage has no BUILD step and consumes a sealed set, and none was given.
    SetRequired,
    /// The stage has a BUILD step, so no set is consumed, and one was given.
    SetNotAllowed,
}

impl fmt::Display for RunHashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunHashError::UnknownStage(name) => write!(f, "the machine has no stage {name}"),
            RunHashError::SetRequired => {
                f.write_str("a stage with no BUILD step is approved with the set it consumes")
            }
            RunHashError::SetNotAllowed => f.write_str(
                "a stage with a BUILD step consumes no set, so none is approved with it",
            ),
        }
    }
}

/// The hash of a run: the machine, the stage, the selection, the revision, the stage's
/// platform, the build stamp (version, build number and source commit; the timestamp is not
/// covered) and, only for a stage with no BUILD step, the sealed set it consumes. It covers
/// no observed state, no run id, no attempt and no ledger.
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
    let consumes = builds(found).next().is_none();
    let set = match (consumes, sealed) {
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

fn stamped(hash: Framed, stamp: &BuildStamp) -> Framed {
    let hash = text(hash, &stamp.version);
    text(number(hash, stamp.build_number), &stamp.source_commit)
}

/// The hash of what a BUILD step builds from: the platform, the revision, the build stamp
/// without its timestamp and, for the step's artifacts and everything they need (in
/// dependency order), each declaration. Editing another stage, a channel or a validation does
/// not change it; editing a builder or an artifact does.
pub fn build_inputs_hash(
    machine: &Checked,
    build: &Build,
    revision: &str,
    stamp: &BuildStamp,
) -> Hash256 {
    let mut hash = text(Framed::new(BUILD_INPUTS_DOMAIN), &build.platform);
    hash = text(hash, revision);
    hash = stamped(hash, stamp);
    let declarations = closure(machine, &build.artifacts);
    hash = number(hash, declarations.len());
    for decl in declarations {
        let builder = decl.builder.clone().unwrap_or_default();
        hash = text(hash, &decl.name);
        hash = number(hash, decl.r#type);
        hash = text(hash, &builder.executable);
        hash = strings(hash, &builder.args);
        hash = strings(hash, &decl.needs);
        hash = number(hash, u8::from(decl.independent_version));
    }
    hash.finish()
}

/// The declarations of `roots` and of everything they need, in dependency order.
fn closure<'a>(machine: &'a Checked, roots: &[String]) -> Vec<&'a ArtifactDecl> {
    let mut wanted: BTreeSet<&str> = BTreeSet::new();
    let mut todo: Vec<&str> = roots.iter().map(String::as_str).collect();
    while let Some(name) = todo.pop() {
        if wanted.insert(name)
            && let Some(decl) = machine.artifact(name)
        {
            todo.extend(decl.needs.iter().map(String::as_str));
        }
    }
    let deps: BTreeMap<&str, BTreeSet<&str>> = wanted
        .iter()
        .filter_map(|name| machine.artifact(name))
        .map(|decl| {
            (
                decl.name.as_str(),
                decl.needs.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    order(&deps)
        .unwrap_or_else(|rest| rest)
        .into_iter()
        .filter_map(|name| machine.artifact(name))
        .collect()
}
