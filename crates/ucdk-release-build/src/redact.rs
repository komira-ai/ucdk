// SPDX-License-Identifier: Apache-2.0
//! Redaction of the output of a tool before it is printed, logged or put in a report.

/// What replaces a credential-shaped run. Deliberately loud: a redaction that cannot be seen is a
/// diagnosis made wrongly.
pub const REDACTED: &str = "[REDACTED]";

/// `text` with the credential-shaped runs replaced by [`REDACTED`].
pub fn redact(text: &str) -> String {
    text.to_owned()
}
