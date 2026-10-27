// SPDX-License-Identifier: Apache-2.0
//! Fixtures shared by the tests of the fakes: a coordinate set, a federated repository, and a
//! registry with the source that issues its grants, both on one manual clock.

#![allow(dead_code)]

use std::collections::BTreeMap;

use ucdk_core::ManualClock;
use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::Held;
use ucdk_release_fake::{FakeGrantSource, FakeRegistry, JobClaims};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{
    ArtifactType, Coordinate, Federated, PushIdentity, Repository, Stamped, Visibility,
};
use ucdk_release_seams::{Blob, Grant, GrantRequest, GrantSource, StageItem};

pub const LOCATION: &str = "registry/files";

pub fn coordinate(name: &str) -> Coordinate {
    Coordinate {
        channel: "stable".to_owned(),
        r#type: ArtifactType::File as i32,
        name: name.to_owned(),
        version: "1.2.3".to_owned(),
        platform: "linux-x86_64".to_owned(),
    }
}

pub fn held() -> Held {
    Held {
        digest: String::new(),
        stamp: Some(Stamped {
            build_number: 41,
            source_commit: "abc123".to_owned(),
        }),
        exceptional: false,
    }
}

/// The repository the grants are for: federated, with three distinct scope names.
pub fn repository() -> Repository {
    Repository {
        r#type: ArtifactType::File as i32,
        location: LOCATION.to_owned(),
        visibility: Visibility::Private as i32,
        push: Some(PushIdentity {
            kind: Some(Kind::Federated(Federated {
                subject: "release".to_owned(),
                audience: "registry".to_owned(),
                exceptional_subject: "exceptional".to_owned(),
                environment: "publish".to_owned(),
                exceptional_environment: "publish-exceptional".to_owned(),
                pinned_claims: BTreeMap::new(),
                stage_scope: "stage".to_owned(),
                commit_scope: "commit".to_owned(),
                read_scope: "read".to_owned(),
            })),
        }),
    }
}

pub fn claims() -> JobClaims {
    JobClaims {
        subject: "release".to_owned(),
        audience: "registry".to_owned(),
        environment: "publish".to_owned(),
        pinned: BTreeMap::new(),
    }
}

/// The names, bytes and held of a set of coordinates to publish.
pub struct Set {
    pub coords: Vec<Coordinate>,
    pub bytes: Vec<Vec<u8>>,
    pub held: Held,
}

impl Set {
    /// One coordinate per name, whose bytes are the name.
    pub fn of(names: &[&str]) -> Self {
        Self {
            coords: names.iter().map(|n| coordinate(n)).collect(),
            bytes: names.iter().map(|n| n.as_bytes().to_vec()).collect(),
            held: held(),
        }
    }

    pub fn blobs(&self) -> Vec<Blob<'_>> {
        self.coords
            .iter()
            .zip(&self.bytes)
            .map(|(c, bytes)| Blob {
                name: c.name.as_str(),
                bytes,
            })
            .collect()
    }

    pub fn items<'a>(&'a self, blobs: &'a [Blob<'a>]) -> Vec<StageItem<'a>> {
        self.coords
            .iter()
            .zip(blobs)
            .map(|(coordinate, file)| StageItem {
                coordinate,
                file,
                held: &self.held,
            })
            .collect()
    }
}

/// A registry and the source of its grants on one clock.
pub struct World {
    pub clock: ManualClock,
    pub registry: FakeRegistry,
    pub source: FakeGrantSource,
    pub repository: Repository,
}

impl World {
    pub fn new() -> Self {
        let clock = ManualClock::at(1_000_000);
        Self::with(&clock, FakeRegistry::new(&clock, LOCATION))
    }

    pub fn with(clock: &ManualClock, registry: FakeRegistry) -> Self {
        Self {
            clock: clock.clone(),
            registry,
            source: FakeGrantSource::new(clock, claims()),
            repository: repository(),
        }
    }

    /// A grant for `phase` covering `coords`.
    pub fn grant(&mut self, phase: GrantPhase, coords: &[Coordinate]) -> Grant {
        self.source
            .grant(&GrantRequest {
                repository: &self.repository,
                phase,
                coordinates: coords,
                exceptional: false,
            })
            .expect("the source issues a grant")
    }
}
