// SPDX-License-Identifier: Apache-2.0
//! Grants: what a registry call carries, and the source that issues them.

use ucdk_release::codec::GrantPhase;
use ucdk_release_proto::v1::{Coordinate, Repository};

use crate::SeamError;

/// What a registry call carries. It is opaque: neither `Debug` nor `Display`, not `Clone`, and
/// its secret is overwritten when it is dropped. It carries only the secret and the expiry the
/// issuer stated; [`Grant::none`] is the anonymous grant of a read that needs none.
pub struct Grant {
    secret: Vec<u8>,
    expires_ms: Option<u64>,
}

impl Grant {
    /// No grant: an anonymous read, or a write to a channel that asks for none.
    pub fn none() -> Self {
        Self {
            secret: Vec::new(),
            expires_ms: None,
        }
    }

    /// A grant the issuer states is good until `expires_ms`.
    pub fn issued(secret: Vec<u8>, expires_ms: u64) -> Self {
        Self {
            secret,
            expires_ms: Some(expires_ms),
        }
    }

    pub fn is_none(&self) -> bool {
        self.expires_ms.is_none()
    }

    /// The expiry the issuer stated, in the issuer's milliseconds; `None` for no grant.
    pub fn expires_ms(&self) -> Option<u64> {
        self.expires_ms
    }

    /// The bytes a registry call presents. Only a publisher reads them.
    pub fn secret(&self) -> &[u8] {
        &self.secret
    }
}

impl Drop for Grant {
    fn drop(&mut self) {
        self.secret.fill(0);
    }
}

/// What a grant must cover.
pub struct GrantRequest<'a> {
    /// Its `push` says whose identity, which audience and which scope names.
    pub repository: &'a Repository,
    pub phase: GrantPhase,
    /// At most `Publisher::grant_limit`: the coordinates of one Publish action (stage and
    /// commit), or of one read in this repository (read).
    pub coordinates: &'a [Coordinate],
    /// Use the exceptional subject and environment.
    pub exceptional: bool,
}

pub trait GrantSource: Send {
    /// An `Anonymous` push identity answers [`Grant::none`] with no exchange.
    fn grant(&mut self, req: &GrantRequest<'_>) -> Result<Grant, SeamError>;
}
