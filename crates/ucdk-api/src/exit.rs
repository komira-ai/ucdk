// SPDX-License-Identifier: Apache-2.0
//! The one table from what a run ended with to its exit class. The command line prints the code
//! as its exit status; an SDK returns the class as a field of the result.

use ucdk_engine::{EngineError, Failure, Outcome, Refusal, Report};
use ucdk_provider::ProviderError;

use crate::v1::ExitClass;

impl ExitClass {
    /// The exit code of the class: 0 done, 2 usage, 3 refused, 4 failed with no effect, 6
    /// failed after changing something.
    pub fn code(self) -> u8 {
        match self {
            Self::Done => 0,
            Self::Usage => 2,
            Self::Refused => 3,
            Self::FailedNoEffect => 4,
            Self::FailedChanged => 6,
        }
    }
}

/// How an outcome ends a command. A plan, and a run that finished (even one that left an object
/// converging or still failed, which its report says), are done.
///
/// Every outcome and every refusal is named here, so a new variant does not compile until it
/// has a class.
pub fn exit_class(outcome: &Outcome) -> ExitClass {
    match outcome {
        Outcome::Planned(_) | Outcome::Done(_) => ExitClass::Done,
        Outcome::Refused(why) => match why {
            Refusal::Conflicts(_)
            | Refusal::EmptyScope
            | Refusal::PlanHashMismatch { .. }
            | Refusal::PlanChanged { .. }
            | Refusal::UnsupportedStampMode(_) => ExitClass::Refused,
        },
        Outcome::Failed(failure) => failure_class(failure),
    }
}

/// The class of a run that stopped. Failed with no effect (safe to repeat) when nothing was
/// done and either nothing was started or the provider's error says the call changed nothing
/// (a refusal, a permanent or a transient error); otherwise failed after changing something,
/// and the world is planned again before running again.
pub fn failure_class(failure: &Failure) -> ExitClass {
    let safe_error = matches!(
        &failure.error,
        EngineError::Provider(
            ProviderError::Refused(_) | ProviderError::Permanent(_) | ProviderError::Transient(_)
        )
    );
    let nothing_done = failure.report == Report::default();
    if nothing_done && (failure.at.is_none() || safe_error) {
        ExitClass::FailedNoEffect
    } else {
        ExitClass::FailedChanged
    }
}
