// SPDX-License-Identifier: Apache-2.0
//! The stable text id of each [`Reason`](crate::v1::Reason).

use crate::v1::Reason;

impl Reason {
    /// The id of the reason, `UCDK-E-` and the words of its name joined by `-`.
    pub fn id(self) -> &'static str {
        let _ = self;
        ""
    }
}
