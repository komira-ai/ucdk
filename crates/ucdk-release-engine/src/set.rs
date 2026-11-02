// SPDX-License-Identifier: Apache-2.0
//! Reading the sealed set a validation is about: from the `stage` record of the stage that built
//! it, through the set document in the store, checked against the record. Nothing in the
//! document is trusted: its hash is recomputed from its members, it must be the hash the
//! record names, and its scope (the revision, the platform and the producing stage and
//! environment) must be the one it was read for.

use ucdk_core::Hash256;
use ucdk_release::machine::{builds, needed_builds, stage_environment};
use ucdk_release::set_document;
use ucdk_release::store::{SetScope, StoreKey};
use ucdk_release_proto::v1::{Reason, ReleaseSet, Stage, Verdict};
use ucdk_release_seams::SeamError;

use crate::engine::Engine;
use crate::outcome::{Finished, Flow};
use crate::records::Records;

/// The file of a set document inside its entry of the store, under the key
/// `<stage>/<platform>/<revision>/<generation>/set`.
pub const SET_FILE: &str = "set.pb";

fn stop(reason: Reason, message: String) -> Finished {
    Finished::stopped(Verdict::Refused, reason, message)
}

impl Engine<'_> {
    /// The set that a validation of `stage` is about: the set `stage` built when it holds a
    /// Build action, and otherwise the set of the stage it needs that does.
    pub(crate) fn load_set(
        &mut self,
        records: &Records,
        stage: &Stage,
    ) -> Flow<(ReleaseSet, Hash256)> {
        let needed = needed_builds(self.machine.machine(), stage);
        let owner = if builds(stage).next().is_some() {
            stage
        } else {
            needed.first().copied().unwrap_or(stage)
        };
        let Some(record) = records.stage(&owner.name) else {
            return Err(stop(
                Reason::StageNotAdmitted,
                format!(
                    "the run ledger has no record of the stage {}, which holds the set",
                    owner.name
                ),
            ));
        };
        let (Some(hash), Some(generation)) = (&record.set, &record.generation) else {
            return Err(stop(
                Reason::SetHash,
                format!("the stage {} has not sealed a set", owner.name),
            ));
        };
        let scope = SetScope {
            stage: owner.name.clone(),
            platform: record.platform.clone().unwrap_or_default(),
            revision: self.run.revision.to_owned(),
            generation: generation.clone(),
        };
        let key = StoreKey::set(&scope).map_err(|e| stop(Reason::SetHash, e.to_string()))?;
        let bytes = match self.seams.store.get(&key, SET_FILE) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => {
                return Err(stop(
                    Reason::SetHash,
                    format!("the store holds no set document at {}", key.as_str()),
                ));
            }
            Err(SeamError::Refused(reason)) => {
                return Err(stop(reason, format!("the store refused {}", key.as_str())));
            }
            Err(other) => {
                return Err(Finished::stopped(
                    Verdict::Indeterminate,
                    Reason::SetHash,
                    format!(
                        "the set document at {} could not be read: {other}",
                        key.as_str()
                    ),
                ));
            }
        };
        let set = set_document::read(&bytes).map_err(|e| stop(e.reason(), e.to_string()))?;
        if set.set_hash != hash.as_str() {
            return Err(stop(
                Reason::SetHash,
                format!(
                    "the set document at {} has hash {}, and the stage record names {hash}",
                    key.as_str(),
                    set.set_hash
                ),
            ));
        }
        // A document is hashed over its members and its scope, so a self-consistent document
        // of another scope passes the checks above; the scope is the caller's to compare.
        let producer = set.producer.clone().unwrap_or_default();
        let differs: Vec<String> = [
            ("revision", set.revision.as_str(), scope.revision.as_str()),
            ("platform", set.platform.as_str(), scope.platform.as_str()),
            (
                "producing stage",
                producer.stage.as_str(),
                owner.name.as_str(),
            ),
            (
                "producing environment",
                producer.environment.as_str(),
                stage_environment(owner),
            ),
        ]
        .into_iter()
        .filter(|(_, found, wanted)| found != wanted)
        .map(|(name, found, wanted)| format!("{name} {found:?}, not {wanted:?}"))
        .collect();
        if !differs.is_empty() {
            return Err(stop(
                Reason::SetHash,
                format!(
                    "the set document at {} is of another scope: {}",
                    key.as_str(),
                    differs.join("; ")
                ),
            ));
        }
        Ok((set, hash.clone()))
    }
}
