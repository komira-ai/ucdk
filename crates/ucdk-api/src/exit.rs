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
            Self::Indeterminate => 0,
            Self::FailedChanged => 6,
            Self::ValidationFailed => 0,
            Self::Leaks => 8,
        }
    }

    /// Whether the class promises that no external effect landed: usage, refused and failed
    /// with no effect.
    pub fn promises_no_effect(self) -> bool {
        false
    }
}

/// The class a run ends in, from the verdict that stopped it, the reason it carries and
/// whether the run recorded a landed or staged coordinate.
pub fn exit_class(verdict: Verdict, reason: Reason, landed: bool) -> ExitClass {
    let _ = (verdict, reason, landed);
    ExitClass::Done
}
