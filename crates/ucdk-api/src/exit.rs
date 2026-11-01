// SPDX-License-Identifier: Apache-2.0
//! The table of exit classes. The command line prints the code as its exit status; an SDK
//! returns the class as a field of the result.

use ucdk_release_proto::v1::{Reason, Verdict};

use crate::v1::ExitClass;

impl ExitClass {
    /// The exit code of the class: 0 done, 2 usage, 3 refused, 4 failed with no effect, 5
    /// indeterminate, 6 failed after changing something, 7 a validation failed, 8 a sweep found
    /// unaccounted objects.
    pub fn code(self) -> u8 {
        match self {
            Self::Done => 0,
            Self::Usage => 2,
            Self::Refused => 3,
            Self::FailedNoEffect => 4,
            Self::Indeterminate => 5,
            Self::FailedChanged => 6,
            Self::ValidationFailed => 7,
            Self::Leaks => 8,
        }
    }

    /// Whether the class promises that no external effect landed: usage, refused and failed
    /// with no effect. A report that lists a landed or staged coordinate never carries one of
    /// these.
    pub fn promises_no_effect(self) -> bool {
        matches!(self, Self::Usage | Self::Refused | Self::FailedNoEffect)
    }
}

/// The class a run ends in, from the verdict that stopped it, the reason it carries and whether
/// the run recorded a landed or staged coordinate.
///
/// The answer is defined for every input. `landed` belongs to the whole run, not to the action
/// that stopped it, so a refusal, a failure or an indeterminate answer after an earlier
/// coordinate landed is class 6 and never 3, 4 or 5.
///
/// - `SUCCEEDED`, `NOOP` and `SUPERSEDED` are done.
/// - `VALIDATION_FAILED` is class 7, landed or not: a validation reads what a step produced.
/// - A lost or an unconfirmed lease is class 6 for every other verdict, even when nothing
///   landed, because the run's `landed` cannot rule out that
///   another writer holds the fence and has changed something.
/// - A done verdict wins over any reason, so `SUCCEEDED` with `LEASE_LOST` is class 0. A
///   producer must never emit that pair; it is answered rather than refused only so that the
///   function stays total.
/// - A verdict nobody set is a bug of the producer and is class 6: it never claims that
///   nothing changed.
///
/// Usage (2) is a fault of the request and has no verdict, and leaks (8) has no producer in a
/// release, so neither is an answer of this function.
pub fn exit_class(verdict: Verdict, reason: Reason, landed: bool) -> ExitClass {
    if matches!(
        verdict,
        Verdict::Succeeded | Verdict::Noop | Verdict::Superseded
    ) {
        return ExitClass::Done;
    }
    if matches!(reason, Reason::LeaseLost | Reason::LeaseUnconfirmed) {
        return ExitClass::FailedChanged;
    }
    match (verdict, landed) {
        (Verdict::ValidationFailed, _) => ExitClass::ValidationFailed,
        (Verdict::Refused, false) => ExitClass::Refused,
        (Verdict::Failed, false) => ExitClass::FailedNoEffect,
        (Verdict::Indeterminate, false) => ExitClass::Indeterminate,
        (
            Verdict::Unspecified
            | Verdict::Succeeded
            | Verdict::Noop
            | Verdict::Superseded
            | Verdict::Refused
            | Verdict::Failed
            | Verdict::Indeterminate
            | Verdict::Partial
            | Verdict::Interrupted,
            _,
        ) => ExitClass::FailedChanged,
    }
}
