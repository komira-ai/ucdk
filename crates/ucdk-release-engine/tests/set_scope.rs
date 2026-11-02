// SPDX-License-Identifier: Apache-2.0
//! A set document is read for a scope: the run's revision, the platform of the stage record and
//! the stage and environment that build it. A document that is sealed and self-consistent, and
//! whose hash is the one the stage record names, is still refused when its scope is another,
//! before any validation record is written.

mod common;

use std::collections::BTreeMap;

use common::{GENERATION, PLATFORM, REVISION, Rig};
use ucdk_core::Hash256;
use ucdk_ledger::State;
use ucdk_release::{codec, set_document};
use ucdk_release_proto::v1::{Member, Producer, Reason, ReleaseSet, Verdict};

/// Seals the example's two packages with `edit` made to the set before it is sealed, and
/// records it as the set of the stage `build`.
fn seal_with(rig: &mut Rig, edit: impl FnOnce(&mut ReleaseSet)) {
    let members = ["ucdk-cli", "ucdk-lib"]
        .iter()
        .map(|name| Member {
            manifest: Some(common::manifest(name, &BTreeMap::new())),
        })
        .collect();
    let mut set = ReleaseSet {
        schema_version: 1,
        revision: REVISION.to_owned(),
        platform: PLATFORM.to_owned(),
        members,
        set_hash: String::new(),
        producer: Some(Producer {
            stage: "build".to_owned(),
            environment: "build".to_owned(),
        }),
        produced_by: None,
    };
    edit(&mut set);
    let set = set_document::seal(set);
    rig.put_set(&set, GENERATION);
    rig.ledger.seed(
        &codec::Record::Stage(codec::Stage {
            stage: "build".to_owned(),
            attempt: 1,
            platform: Some(PLATFORM.to_owned()),
            set: Some(Hash256::from_hex(&set.set_hash).expect("a hash")),
            generation: Some(GENERATION.to_owned()),
            confirmed: false,
        }),
        State::Intent,
    );
}

fn refused(name: &str, edit: impl FnOnce(&mut ReleaseSet), differs: &str) {
    let mut rig = Rig::new(name);
    seal_with(&mut rig, edit);
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(
        (outcome.verdict, outcome.reason),
        (Verdict::Refused, Reason::SetHash),
        "{name}: {}",
        outcome.message
    );
    assert!(
        outcome.message.contains("is of another scope: ") && outcome.message.ends_with(differs),
        "{name}: {}",
        outcome.message
    );
    assert!(
        rig.ledger.validation("build", "closure").is_none(),
        "{name}: no validation record was written"
    );
}

#[test]
fn a_set_of_another_revision_is_refused() {
    let other = "fedcba9876543210fedcba9876543210fedcba98";
    refused(
        "scope_revision",
        |s| s.revision = other.to_owned(),
        &format!("revision {other:?}, not {REVISION:?}"),
    );
}

#[test]
fn a_set_of_another_platform_is_refused() {
    refused(
        "scope_platform",
        |s| s.platform = "linux-arm64".to_owned(),
        &format!("platform \"linux-arm64\", not {PLATFORM:?}"),
    );
}

#[test]
fn a_set_another_stage_or_environment_produced_is_refused() {
    refused(
        "scope_stage",
        |s| s.producer.as_mut().expect("a producer").stage = "other".to_owned(),
        "producing stage \"other\", not \"build\"",
    );
    refused(
        "scope_environment",
        |s| s.producer.as_mut().expect("a producer").environment = "prod".to_owned(),
        "producing environment \"prod\", not \"build\"",
    );
    refused(
        "scope_no_producer",
        |s| s.producer = None,
        "producing stage \"\", not \"build\"; producing environment \"\", not \"build\"",
    );
}

#[test]
fn a_set_of_its_own_scope_is_read() {
    let mut rig = Rig::new("scope_own");
    seal_with(&mut rig, |_| {});
    let outcome = rig.handle("build", &["validation:closure"]);
    assert_eq!(outcome.verdict, Verdict::Succeeded, "{}", outcome.message);
}
