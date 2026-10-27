// SPDX-License-Identifier: Apache-2.0
//! The grants the publisher rigs issue: the issuer of the fake, and the scope a grant carries.

use ucdk_core::ManualClock;
use ucdk_release::codec::GrantPhase;
use ucdk_release_fake::{FakeGrantSource, JobClaims};
use ucdk_release_proto::v1::{Coordinate, Repository};
use ucdk_release_seams::kit::matching_job;
use ucdk_release_seams::{Grant, GrantRequest, GrantSource};

use super::secret::texts;

/// A grant of `phase` for `coordinates` in `repository`, good now.
pub(super) fn issue(
    clock: &ManualClock,
    repository: &Repository,
    phase: GrantPhase,
    coordinates: &[Coordinate],
) -> Grant {
    let job = matching_job();
    let claims = JobClaims {
        subject: job.subject,
        audience: job.audience,
        environment: job.environment,
        pinned: job.pinned,
    };
    let mut source = FakeGrantSource::new(clock, claims);
    source
        .grant(&GrantRequest {
            repository,
            phase,
            coordinates,
            exceptional: false,
        })
        .expect("the issuer gives a grant")
}

/// The scope of a grant: the first text its issuer wrote into the secret.
pub(super) fn scope_of(g: &Grant) -> String {
    texts(g.secret()).into_iter().next().unwrap_or_default()
}
