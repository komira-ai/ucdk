// SPDX-License-Identifier: Apache-2.0
//! Why a step of a run stopped.

use std::fmt;

use ucdk_ledger::LedgerError;
use ucdk_plan::PlanHash;
use ucdk_provider::{NodePath, ProviderError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// A provider call failed. Whether the call changed anything follows from its class
    /// ([`ProviderError`]); the ledger still records the intent.
    Provider(ProviderError),
    /// A ledger read or write failed.
    Ledger(LedgerError),
    /// The run's lease was taken over, released or lapsed, so the ledger refused its write (or
    /// its renewal). The run stopped; whatever it had begun is resolved by the next holder.
    LeaseLost,
    /// The ledger lease may still be held: taking it, or giving it back, failed with an answer
    /// that does not say whether the ledger changed. Until it lapses, a rerun is refused for
    /// every holder but this run's, which takes the lease again at once. `run` is the error the
    /// run had already stopped with, when the failure was in giving the lease back.
    LeaseUnconfirmed {
        why: String,
        run: Option<Box<EngineError>>,
    },
    /// The ledger stayed stale through every retry: another run keeps writing it.
    LedgerBusy,
    /// An object found while the run was going is not ours (no stamp of the node, or a
    /// different physical id than the record's), or has no record to act on.
    NotOurs(NodePath),
    /// An object that was expected to be gone is there.
    StillPresent(NodePath),
    /// A read shows no object, but the run that created it never read it back: the read may
    /// only be lagging, so the record stays until the object is seen.
    Unwitnessed(NodePath),
    /// A node reads an output of a resource whose record is confirmed and holds no such
    /// output: the provider does not report it (or reports it as a secret), so the value
    /// does not exist.
    OutputUnknown {
        path: NodePath,
        resource: String,
        output: String,
    },
}

impl From<ProviderError> for EngineError {
    fn from(e: ProviderError) -> Self {
        Self::Provider(e)
    }
}

impl From<LedgerError> for EngineError {
    fn from(e: LedgerError) -> Self {
        match e {
            LedgerError::Fenced | LedgerError::LeaseLapsed => Self::LeaseLost,
            other => Self::Ledger(other),
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(e) => write!(f, "provider: {e}"),
            Self::Ledger(e) => write!(f, "ledger: {e}"),
            Self::LeaseLost => f.write_str(
                "the ledger lease was lost (it lapsed, or another run took it): this run stopped \
                 at its last whole write",
            ),
            Self::LeaseUnconfirmed { why, run } => {
                write!(
                    f,
                    "the ledger lease may still be held ({why}): run again as the same holder, \
                     or wait for the lease to lapse"
                )?;
                if let Some(run) = run {
                    write!(f, "; the run had stopped: {run}")?;
                }
                Ok(())
            }
            Self::LedgerBusy => f.write_str("the ledger kept changing under the run"),
            Self::NotOurs(p) => write!(f, "{p}: the object is not ours (stamp and record)"),
            Self::StillPresent(p) => write!(f, "{p}: the object is there when it should be gone"),
            Self::Unwitnessed(p) => write!(
                f,
                "{p}: no object is read, but the record is an intent that no read has confirmed"
            ),
            Self::OutputUnknown {
                path,
                resource,
                output,
            } => write!(
                f,
                "{path}: `{resource}` is applied and reports no {output} for it to read"
            ),
        }
    }
}

impl std::error::Error for EngineError {}

/// A plan hash, as a short text for a message.
pub(crate) fn short(hash: &PlanHash) -> &str {
    &hash.as_str()[..12]
}
