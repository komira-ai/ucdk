// SPDX-License-Identifier: Apache-2.0
//! The issuer of grants. It compares the job's claims with the repository's declaration before
//! it exchanges, issues grants whose expiry is the shared clock plus a lifetime, and records
//! every exchange it made.

use std::collections::BTreeMap;

use ucdk_core::{Clock, ManualClock};
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{Coordinate, Reason};
use ucdk_release_seams::{Grant, GrantRequest, GrantSource, SeamError};

use crate::token::{coordinate_key, encode};

/// What the job's identity token says: the claims the issuer compares with the declaration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JobClaims {
    pub subject: String,
    pub audience: String,
    pub pinned: BTreeMap<String, String>,
}

/// One exchange that reached the issuer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exchange {
    pub phase: GrantPhase,
    pub scope: String,
    pub coordinates: Vec<String>,
    pub subject: String,
    pub audience: String,
    pub repository: String,
    /// Set when a fault refused it.
    pub fault: Option<GrantFault>,
}

pub struct FakeGrantSource {
    clock: ManualClock,
    claims: JobClaims,
    lifetime_ms: u64,
    offset_ms: i64,
    limit: usize,
    /// One-shot refusals: the phase they apply to (any when `None`) and the fault.
    refusals: Vec<(Option<GrantPhase>, GrantFault)>,
    exchanges: Vec<Exchange>,
}

impl FakeGrantSource {
    /// A source for a job that presents `claims`, with grants good for ten minutes.
    pub fn new(clock: &ManualClock, claims: JobClaims) -> Self {
        Self {
            clock: clock.clone(),
            claims,
            lifetime_ms: 600_000,
            offset_ms: 0,
            limit: usize::MAX,
            refusals: Vec::new(),
            exchanges: Vec::new(),
        }
    }

    /// The most coordinates one grant may cover.
    #[must_use]
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// The next exchange of `phase` is refused with `fault`; later ones are not.
    pub fn refuse(&mut self, phase: GrantPhase, fault: GrantFault) {
        self.refusals.push((Some(phase), fault));
    }

    /// The next exchange, of any phase, finds its identity token already used.
    pub fn identity_token_already_used(&mut self) {
        self.refusals.push((None, GrantFault::Used));
    }

    /// Grants live this many milliseconds, so one can be shorter than the margin.
    pub fn lifetime(&mut self, ms: u64) {
        self.lifetime_ms = ms;
    }

    /// The expiry a grant states is on a clock this many milliseconds ahead of the shared one
    /// (behind, when negative).
    pub fn issuer_clock_offset(&mut self, ms: i64) {
        self.offset_ms = ms;
    }

    /// Every exchange that reached the issuer, in order.
    pub fn exchanges(&self) -> &[Exchange] {
        &self.exchanges
    }
}

impl GrantSource for FakeGrantSource {
    fn grant(&mut self, req: &GrantRequest<'_>) -> Result<Grant, SeamError> {
        let Some(Kind::Federated(declared)) =
            req.repository.push.as_ref().and_then(|p| p.kind.as_ref())
        else {
            return Ok(Grant::none());
        };
        if req.coordinates.len() > self.limit {
            return Err(SeamError::Refused(Reason::Credential));
        }
        let subject = if req.exceptional {
            &declared.exceptional_subject
        } else {
            &declared.subject
        };
        if *subject != self.claims.subject
            || declared.audience != self.claims.audience
            || declared.pinned_claims != self.claims.pinned
        {
            return Err(SeamError::GrantRefused(GrantFault::Denied));
        }
        let scope = match req.phase {
            GrantPhase::Stage => &declared.stage_scope,
            GrantPhase::Commit => &declared.commit_scope,
            GrantPhase::Read => &declared.read_scope,
        };
        let refusal = self
            .refusals
            .iter()
            .position(|(phase, _)| phase.is_none_or(|p| p == req.phase));
        let fault = refusal.map(|at| self.refusals.remove(at).1);
        self.exchanges.push(Exchange {
            phase: req.phase,
            scope: scope.clone(),
            coordinates: req.coordinates.iter().map(coordinate_key).collect(),
            subject: subject.clone(),
            audience: declared.audience.clone(),
            repository: req.repository.location.clone(),
            fault,
        });
        if let Some(fault) = fault {
            return Err(SeamError::GrantRefused(fault));
        }
        let stated = i128::from(self.clock.now_millis())
            + i128::from(self.lifetime_ms)
            + i128::from(self.offset_ms);
        let expires = u64::try_from(stated.max(0)).unwrap_or(u64::MAX);
        let covered: &[Coordinate] = req.coordinates;
        Ok(Grant::issued(
            encode(scope, &req.repository.location, covered),
            expires,
        ))
    }
}
