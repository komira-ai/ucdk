// SPDX-License-Identifier: Apache-2.0
//! The one table from what a run ended with to its exit class. The command line prints the code
//! as its exit status; an SDK returns the class as a field of the result.

use ucdk_engine::{EngineError, Failure, Outcome, Refusal, Report};
use ucdk_provider::ProviderError;

use crate::v1::ExitClass;

impl ExitClass {
    /// The exit code of the class: 0 done, 2 usage, 3 refused, 4 failed with no effect, 6
    /// failed after changing something, 8 leaks found.
    pub fn code(self) -> u8 {
        match self {
            Self::Done => 0,
            Self::Usage => 2,
            Self::Refused => 3,
            Self::FailedNoEffect => 4,
            Self::FailedChanged => 6,
            Self::Leaks => 8,
        }
    }
}

/// How an outcome ends a command. A plan, a status (whatever it found: a missing object is a finding, not a failure), and a
/// run that finished (even one that left an object
/// converging or still failed, which its report says), are done. A leak sweep is done when it
/// found none and `Leaks` when it found some; an apply that finds leaks is still done, because
/// its report carries them.
///
/// Every outcome and every refusal is named here, so a new variant does not compile until it
/// has a class.
pub fn exit_class(outcome: &Outcome) -> ExitClass {
    match outcome {
        Outcome::Planned(_) | Outcome::Done(_) | Outcome::Status(_) => ExitClass::Done,
        Outcome::Leaks(leaks) if leaks.is_empty() => ExitClass::Done,
        Outcome::Leaks(_) => ExitClass::Leaks,
        Outcome::Refused(why) => match why {
            Refusal::Conflicts(_)
            | Refusal::EmptyScope
            | Refusal::PlanHashMismatch { .. }
            | Refusal::PlanChanged { .. }
            | Refusal::UnsupportedStampMode(_)
            | Refusal::UnhostedKinds(_)
            | Refusal::LeaseHeld { .. } => ExitClass::Refused,
        },
        Outcome::Failed(failure) => failure_class(failure),
    }
}

/// The class of a run that stopped. Failed with no effect (safe to repeat) when nothing was
/// done and either nothing was started or the provider's error says the call changed nothing
/// (a refusal, a permanent or a transient error); otherwise failed after changing something,
/// and the world is planned again before running again. A run that may still hold the ledger's
/// lease ([`EngineError::LeaseUnconfirmed`]) is always the second kind.
pub fn failure_class(failure: &Failure) -> ExitClass {
    let safe_error = matches!(
        &failure.error,
        EngineError::Provider(
            ProviderError::Refused(_) | ProviderError::Permanent(_) | ProviderError::Transient(_)
        )
    );
    let nothing_done = failure.report == Report::default();
    if matches!(failure.error, EngineError::LeaseUnconfirmed { .. }) {
        // The lease may be held, so a rerun by another holder is refused for a while.
        ExitClass::FailedChanged
    } else if nothing_done && (failure.at.is_none() || safe_error) {
        ExitClass::FailedNoEffect
    } else {
        ExitClass::FailedChanged
    }
}
