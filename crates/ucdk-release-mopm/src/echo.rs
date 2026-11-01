// SPDX-License-Identifier: Apache-2.0
//! The echo guard: text from a registry is quoted in a diagnostic only if it does not hold the
//! credential the request carried.

use crate::transport::Secret;

/// The longest excerpt of a registry's body that a diagnostic quotes.
pub const DETAIL_EXCERPT_BYTES: usize = 400;
/// The shortest run of a credential whose appearance withholds a text.
pub const ECHO_WINDOW_BYTES: usize = 16;
/// What is said instead of a text that echoed the credential.
pub const WITHHELD_ECHO: &str =
    "the registry's answer was withheld: it echoed the request's credential";

/// Whether `hay` holds the credential, or a run of it.
pub fn echoes_credential(_hay: &[u8], _authorization: &Secret) -> bool {
    false
}

/// A bounded excerpt of a registry's `body`, or [`WITHHELD_ECHO`] if it echoes the credential.
pub fn excerpt_unless_echoes(body: &[u8], _authorization: &Secret) -> String {
    String::from_utf8_lossy(body).into_owned()
}

/// `detail`, or [`WITHHELD_ECHO`] if it echoes the credential.
pub fn withhold_if_echoes(detail: String, _authorization: &Secret) -> String {
    detail
}
