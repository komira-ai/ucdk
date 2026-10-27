// SPDX-License-Identifier: Apache-2.0
//! The grant-source rig: the fake issuer, and the stubs that break one promise of an issuer.

use std::time::Duration;

use ucdk_core::{Clock, ManualClock};
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_fake::{FakeGrantSource, JobClaims as FakeClaims};
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::kit::{GrantContent, GrantSetup, GrantSourceRig, matching_job};
use ucdk_release_seams::{Grant, GrantRequest, GrantSource, SeamError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// An anonymous identity is answered with a grant.
    AnonymousGetsAGrant,
    /// A stage grant for two coordinates covers only the first.
    DropsTheLastCoordinate,
    /// The pinned claims of the job are not compared.
    IgnoresPinnedClaims,
    /// The request is never exceptional.
    IgnoresExceptional,
    /// A used identity token is refused as `Denied`.
    UsedIsDenied,
    /// An expired grant is handed out again, as it was.
    ReissuesTheExpiredGrant,
    /// A commit grant for an identity without the scope is issued anyway.
    CommitWithoutScopeIsIssued,
    /// A read grant is issued with the stage scope.
    ReadIsIssuedAsStage,
    /// An unreachable issuer spends the identity token.
    UnreachableSpendsTheToken,
    /// The stated expiry is moved by the offset of the issuer's clock.
    CorrectsTheSkew,
    /// A grant for one coordinate over the limit is issued for the limit.
    AllowsOneOverTheLimit,
    /// The secret of a grant is written into the log.
    LogsTheSecret,
}

pub struct FakeGrantRig {
    clock: ManualClock,
    flaw: Option<Flaw>,
}

impl FakeGrantRig {
    pub fn conforming() -> Self {
        Self {
            clock: ManualClock::at(1_000_000),
            flaw: None,
        }
    }

    pub fn flawed(flaw: Flaw) -> Self {
        Self {
            flaw: Some(flaw),
            ..Self::conforming()
        }
    }
}

pub struct Flawed {
    inner: FakeGrantSource,
    flaw: Option<Flaw>,
    clock: ManualClock,
    limit: usize,
    offset_ms: i64,
    cached: Option<(Vec<u8>, u64)>,
    unreachable_seen: bool,
    log: Vec<u8>,
}

impl GrantSourceRig for FakeGrantRig {
    type Source = Flawed;

    fn source(&self, setup: &GrantSetup) -> Flawed {
        let mut pinned = setup.job.pinned.clone();
        if self.flaw == Some(Flaw::IgnoresPinnedClaims) {
            pinned = matching_job().pinned;
        }
        let claims = FakeClaims {
            subject: setup.job.subject.clone(),
            audience: setup.job.audience.clone(),
            environment: setup.job.environment.clone(),
            pinned,
        };
        let mut inner = FakeGrantSource::new(&self.clock, claims).with_limit(setup.limit);
        inner.lifetime(setup.lifetime_ms);
        inner.issuer_clock_offset(setup.issuer_offset_ms);
        Flawed {
            inner,
            flaw: self.flaw,
            clock: self.clock.clone(),
            limit: setup.limit,
            offset_ms: setup.issuer_offset_ms,
            cached: None,
            unreachable_seen: false,
            log: Vec::new(),
        }
    }

    fn now_ms(&self) -> u64 {
        self.clock.now_millis()
    }

    fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
    }

    fn exchanges(&self, source: &Flawed) -> usize {
        source.inner.exchanges().len()
    }

    fn content(&self, _source: &Flawed, grant: &Grant) -> GrantContent {
        let text = String::from_utf8_lossy(grant.secret()).into_owned();
        let mut lines = text.lines();
        let scope = lines.next().unwrap_or_default().to_owned();
        let repository = lines.next().unwrap_or_default().to_owned();
        let coordinates = lines
            .map(|line| {
                let parts: Vec<&str> = line.split('/').collect();
                Coordinate {
                    channel: parts[0].to_owned(),
                    r#type: parts[1].parse().unwrap_or_default(),
                    name: parts[2].to_owned(),
                    version: parts[3].to_owned(),
                    platform: parts[4].to_owned(),
                }
            })
            .collect();
        GrantContent {
            scope,
            repository,
            coordinates,
        }
    }

    fn refuse_next(&self, source: &mut Flawed, phase: GrantPhase, fault: GrantFault) {
        source.inner.refuse(phase, fault);
    }

    fn log(&self, source: &Flawed) -> Vec<u8> {
        let mut log = format!("{:?}", source.inner.exchanges()).into_bytes();
        log.extend_from_slice(&source.log);
        log
    }
}

impl Flawed {
    fn is(&self, flaw: Flaw) -> bool {
        self.flaw == Some(flaw)
    }
}

impl GrantSource for Flawed {
    fn grant(&mut self, req: &GrantRequest<'_>) -> Result<Grant, SeamError> {
        if self.is(Flaw::ReissuesTheExpiredGrant)
            && let Some((secret, expires)) = &self.cached
            && *expires <= self.clock.now_millis()
        {
            return Ok(Grant::issued(secret.clone(), *expires));
        }
        if self.unreachable_seen {
            self.unreachable_seen = false;
            return Err(SeamError::GrantRefused(GrantFault::Used));
        }
        let mut phase = req.phase;
        if self.is(Flaw::ReadIsIssuedAsStage) && phase == GrantPhase::Read {
            phase = GrantPhase::Stage;
        }
        let mut coordinates = req.coordinates;
        if self.is(Flaw::DropsTheLastCoordinate)
            && phase == GrantPhase::Stage
            && coordinates.len() == 2
        {
            coordinates = &coordinates[..1];
        }
        if self.is(Flaw::AllowsOneOverTheLimit)
            && Some(coordinates.len()) == self.limit.checked_add(1)
        {
            coordinates = &coordinates[..self.limit];
        }
        let result = self.inner.grant(&GrantRequest {
            repository: req.repository,
            phase,
            coordinates,
            exceptional: req.exceptional && !self.is(Flaw::IgnoresExceptional),
        });
        match (self.flaw, result) {
            (Some(Flaw::AnonymousGetsAGrant), Ok(grant)) if grant.is_none() => {
                Ok(Grant::issued(b"x".to_vec(), 1))
            }
            (Some(Flaw::UsedIsDenied), Err(SeamError::GrantRefused(GrantFault::Used))) => {
                Err(SeamError::GrantRefused(GrantFault::Denied))
            }
            (
                Some(Flaw::CommitWithoutScopeIsIssued),
                Err(SeamError::GrantRefused(GrantFault::Denied)),
            ) if req.phase == GrantPhase::Commit => Ok(Grant::issued(b"commit".to_vec(), 1)),
            (
                Some(Flaw::UnreachableSpendsTheToken),
                Err(SeamError::GrantRefused(GrantFault::Unreachable)),
            ) => {
                self.unreachable_seen = true;
                Err(SeamError::GrantRefused(GrantFault::Unreachable))
            }
            (Some(Flaw::CorrectsTheSkew), Ok(grant)) if self.offset_ms != 0 => Ok(Grant::issued(
                grant.secret().to_vec(),
                grant
                    .expires_ms()
                    .unwrap_or_default()
                    .saturating_add_signed(-self.offset_ms),
            )),
            (Some(Flaw::LogsTheSecret), Ok(grant)) => {
                self.log.extend_from_slice(grant.secret());
                Ok(grant)
            }
            (Some(Flaw::ReissuesTheExpiredGrant), Ok(grant)) => {
                self.cached = Some((
                    grant.secret().to_vec(),
                    grant.expires_ms().unwrap_or_default(),
                ));
                Ok(grant)
            }
            (_, result) => result,
        }
    }
}
