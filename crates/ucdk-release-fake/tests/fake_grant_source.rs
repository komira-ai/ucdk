// SPDX-License-Identifier: Apache-2.0
//! The fake grant source: what it compares, what it issues, and what it records.

mod common;

use common::*;
use ucdk_core::ManualClock;
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_fake::FakeGrantSource;
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{Anonymous, PushIdentity, Reason};
use ucdk_release_seams::{GrantRequest, GrantSource, SeamError};

fn ask(
    source: &mut FakeGrantSource,
    phase: GrantPhase,
    exceptional: bool,
    n: usize,
) -> Result<ucdk_release_seams::Grant, SeamError> {
    let coords: Vec<_> = (0..n).map(|i| coordinate(&format!("c{i}"))).collect();
    source.grant(&GrantRequest {
        repository: &repository(),
        phase,
        coordinates: &coords,
        exceptional,
    })
}

fn source() -> (ManualClock, FakeGrantSource) {
    let clock = ManualClock::at(5_000);
    let source = FakeGrantSource::new(&clock, claims());
    (clock, source)
}

#[test]
fn a_differing_claim_is_denied_and_spends_nothing() {
    for edit in [
        (|c: &mut ucdk_release_fake::JobClaims| c.subject = "someone".to_owned())
            as fn(&mut ucdk_release_fake::JobClaims),
        |c| c.audience = "elsewhere".to_owned(),
        |c| {
            c.pinned.insert("ref".to_owned(), "main".to_owned());
        },
    ] {
        let clock = ManualClock::at(0);
        let mut job = claims();
        edit(&mut job);
        let mut source = FakeGrantSource::new(&clock, job);
        assert_eq!(
            ask(&mut source, GrantPhase::Stage, false, 1).err(),
            Some(SeamError::GrantRefused(GrantFault::Denied))
        );
        assert!(source.exchanges().is_empty());
    }
}

#[test]
fn an_unreachable_issuer_is_grant_refused_unreachable() {
    let (_, mut source) = source();
    source.refuse(GrantPhase::Commit, GrantFault::Unreachable);
    assert_eq!(
        ask(&mut source, GrantPhase::Commit, false, 1).err(),
        Some(SeamError::GrantRefused(GrantFault::Unreachable))
    );
    assert_eq!(source.exchanges()[0].fault, Some(GrantFault::Unreachable));
    // The refusal is for one exchange: the next is issued.
    assert!(ask(&mut source, GrantPhase::Commit, false, 1).is_ok());
    assert_eq!(source.exchanges()[1].fault, None);
}

#[test]
fn the_issuer_clock_offset_shifts_the_stated_expiry() {
    let (_, mut source) = source();
    assert_eq!(
        ask(&mut source, GrantPhase::Stage, false, 1)
            .unwrap()
            .expires_ms(),
        Some(5_000 + 600_000)
    );
    source.issuer_clock_offset(30_000);
    assert_eq!(
        ask(&mut source, GrantPhase::Stage, false, 1)
            .unwrap()
            .expires_ms(),
        Some(5_000 + 600_000 + 30_000)
    );
    source.issuer_clock_offset(-1_000_000);
    source.lifetime(10);
    // An expiry before the epoch is the epoch.
    assert_eq!(
        ask(&mut source, GrantPhase::Stage, false, 1)
            .unwrap()
            .expires_ms(),
        Some(0)
    );
}

#[test]
fn a_read_grant_carries_the_read_scope() {
    let (_, mut source) = source();
    for (phase, scope) in [
        (GrantPhase::Stage, "stage"),
        (GrantPhase::Commit, "commit"),
        (GrantPhase::Read, "read"),
    ] {
        ask(&mut source, phase, false, 2).unwrap();
        let last = source.exchanges().last().unwrap();
        assert_eq!((last.phase, last.scope.as_str()), (phase, scope));
        assert_eq!(last.coordinates.len(), 2);
        assert_eq!(last.repository, LOCATION);
        assert_eq!(
            (last.subject.as_str(), last.audience.as_str()),
            ("release", "registry")
        );
    }
}

#[test]
fn an_anonymous_identity_needs_no_exchange() {
    let (_, mut source) = source();
    let mut repo = repository();
    repo.push = Some(PushIdentity {
        kind: Some(Kind::Anonymous(Anonymous {})),
    });
    let grant = source
        .grant(&GrantRequest {
            repository: &repo,
            phase: GrantPhase::Commit,
            coordinates: &[],
            exceptional: false,
        })
        .unwrap();
    assert!(grant.is_none());
    repo.push = None;
    let grant = source
        .grant(&GrantRequest {
            repository: &repo,
            phase: GrantPhase::Commit,
            coordinates: &[],
            exceptional: false,
        })
        .unwrap();
    assert!(grant.is_none());
    assert!(source.exchanges().is_empty());
}

#[test]
fn more_coordinates_than_the_limit_are_refused_before_the_exchange() {
    let clock = ManualClock::at(0);
    let mut source = FakeGrantSource::new(&clock, claims()).with_limit(2);
    assert!(ask(&mut source, GrantPhase::Stage, false, 2).is_ok());
    assert_eq!(
        ask(&mut source, GrantPhase::Stage, false, 3).err(),
        Some(SeamError::Refused(Reason::Credential))
    );
    assert_eq!(source.exchanges().len(), 1);
}

#[test]
fn an_exceptional_run_presents_the_exceptional_subject() {
    let (_, mut source) = source();
    // The job presents the ordinary subject, so an exceptional request does not match it.
    assert_eq!(
        ask(&mut source, GrantPhase::Stage, true, 1).err(),
        Some(SeamError::GrantRefused(GrantFault::Denied))
    );
    let clock = ManualClock::at(0);
    let mut job = claims();
    job.subject = "exceptional".to_owned();
    job.environment = "publish-exceptional".to_owned();
    let mut source = FakeGrantSource::new(&clock, job);
    ask(&mut source, GrantPhase::Stage, true, 1).unwrap();
    assert_eq!(source.exchanges()[0].subject, "exceptional");
}

#[test]
fn a_job_in_the_wrong_environment_is_denied_and_spends_nothing() {
    let clock = ManualClock::at(0);
    // An ordinary run from another environment than the declared one.
    let mut job = claims();
    job.environment = "scratch".to_owned();
    let mut source = FakeGrantSource::new(&clock, job);
    assert_eq!(
        ask(&mut source, GrantPhase::Stage, false, 1).err(),
        Some(SeamError::GrantRefused(GrantFault::Denied))
    );
    // An exceptional run whose subject is right but which runs in the ordinary environment.
    let mut job = claims();
    job.subject = "exceptional".to_owned();
    let mut source = FakeGrantSource::new(&clock, job);
    assert_eq!(
        ask(&mut source, GrantPhase::Stage, true, 1).err(),
        Some(SeamError::GrantRefused(GrantFault::Denied))
    );
    // An ordinary run from the exceptional environment.
    let mut job = claims();
    job.environment = "publish-exceptional".to_owned();
    let mut source = FakeGrantSource::new(&clock, job);
    assert_eq!(
        ask(&mut source, GrantPhase::Stage, false, 1).err(),
        Some(SeamError::GrantRefused(GrantFault::Denied))
    );
    assert!(source.exchanges().is_empty());
}

#[test]
fn a_used_identity_token_is_refused_once_and_a_phase_fault_waits_for_its_phase() {
    let (_, mut source) = source();
    source.refuse(GrantPhase::Commit, GrantFault::Expired);
    source.identity_token_already_used();
    // The fault of the commit phase does not apply to a stage exchange; the one for any phase does.
    assert_eq!(
        ask(&mut source, GrantPhase::Stage, false, 1).err(),
        Some(SeamError::GrantRefused(GrantFault::Used))
    );
    assert!(ask(&mut source, GrantPhase::Stage, false, 1).is_ok());
    assert_eq!(
        ask(&mut source, GrantPhase::Commit, false, 1).err(),
        Some(SeamError::GrantRefused(GrantFault::Expired))
    );
}
