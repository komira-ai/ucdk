// SPDX-License-Identifier: Apache-2.0
//! The table of exit classes. The command line prints the code as its exit status; an SDK
//! returns the class as a field of the result.

use crate::v1::ExitClass;

impl ExitClass {
    /// The exit code of the class: 0 done, 2 usage, 3 refused, 4 failed with no effect, 6
    /// failed after changing something, 8 a sweep found unaccounted objects.
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
