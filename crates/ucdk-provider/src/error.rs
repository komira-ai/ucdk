// SPDX-License-Identifier: Apache-2.0
//! What a provider call can fail with, classified by what the caller may assume happened.

use std::fmt;

/// Why a provider refused a call. A refusal means nothing was changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A create found an object of that kind and name already there.
    AlreadyExists,
    /// An update named an object that is not there.
    NotFound,
    /// The provider does not host this kind.
    Unsupported,
    /// The provider declined the call for another reason.
    Rejected,
}

/// The five outcomes of a failed call. The caller's recovery follows the class: `Refused`
/// changed nothing; `Permanent` will fail again; `Transient` may be retried; `InFlight` and
/// `Unknown` may have changed something, so the caller resolves them by observing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    Refused(Refusal),
    Permanent(String),
    Transient(String),
    InFlight(String),
    Unknown(String),
}

impl ProviderError {
    /// The refusal, when this error is one.
    pub fn refusal(&self) -> Option<Refusal> {
        match self {
            Self::Refused(why) => Some(*why),
            _ => None,
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(why) => write!(f, "refused: {why:?}"),
            Self::Permanent(m) => write!(f, "permanent: {m}"),
            Self::Transient(m) => write!(f, "transient: {m}"),
            Self::InFlight(m) => write!(f, "in flight: {m}"),
            Self::Unknown(m) => write!(f, "unknown: {m}"),
        }
    }
}

impl std::error::Error for ProviderError {}
