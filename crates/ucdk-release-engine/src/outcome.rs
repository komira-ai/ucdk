// SPDX-License-Identifier: Apache-2.0
//! What a request answers.

use ucdk_release_proto::v1::{ActionReport, Reason, ValidationReport, Verdict};

/// The answer to a [`Request`](crate::Request).
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub verdict: Verdict,
    pub reason: Reason,
    pub message: String,
    pub actions: Vec<ActionReport>,
    pub validations: Vec<ValidationReport>,
}

impl Outcome {
    pub(crate) fn halted(verdict: Verdict, reason: Reason, message: impl Into<String>) -> Self {
        Self {
            verdict,
            reason,
            message: message.into(),
            actions: Vec::new(),
            validations: Vec::new(),
        }
    }
}
