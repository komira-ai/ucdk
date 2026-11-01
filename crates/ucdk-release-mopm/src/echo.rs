// SPDX-License-Identifier: Apache-2.0
//! The echo guard: text from a registry is quoted in a diagnostic only if it does not hold the
//! credential the request carried.
//!
//! A registry may answer a refusal with the credential it was sent. A diagnostic reaches a
//! terminal and a log, so such an answer is withheld whole. The credential is held in the shapes
//! a server could echo: the whole `authorization` value, and the part after its scheme. A shape
//! of at least [`ECHO_WINDOW_BYTES`] is matched by any run of that length, which also catches an
//! echo that the server cut short; a shorter one (of four bytes or more) is matched whole.
//!
//! Not held: an echo shorter than the window, and one re-encoded (percent-encoded, escaped in
//! JSON) so that no run of the window length is left whole. A body is checked whole before it is
//! cut to an excerpt, so that the bound never leaves a prefix of an echo to quote.

use std::collections::HashSet;

use crate::transport::Secret;

/// The longest excerpt of a registry's body that a diagnostic quotes.
pub const DETAIL_EXCERPT_BYTES: usize = 400;
/// The shortest run of a credential whose appearance withholds a text. Sixteen bytes of a random
/// token is more than 96 bits; a shorter run would withhold ordinary text.
pub const ECHO_WINDOW_BYTES: usize = 16;
/// What is said instead of a text that echoed the credential.
pub const WITHHELD_ECHO: &str =
    "the registry's answer was withheld: it echoed the request's credential";

/// The shortest shape that is looked for. Anything shorter would match ordinary text, and no
/// real credential is that short.
const MIN_SHAPE_BYTES: usize = 4;

/// The shapes of `authorization` that a server could echo.
fn shapes(authorization: &Secret) -> Vec<&[u8]> {
    let whole = authorization.expose();
    let mut shapes = vec![whole];
    if let Some(space) = whole.iter().position(|&b| b == b' ') {
        shapes.push(&whole[space + 1..]);
    }
    shapes.retain(|shape| shape.len() >= MIN_SHAPE_BYTES);
    shapes
}

/// Whether `hay` holds a shape of `authorization` whole, or any run of [`ECHO_WINDOW_BYTES`] of
/// one that is that long. An empty `authorization` (a request with no credential) is never
/// echoed.
pub fn echoes_credential(hay: &[u8], authorization: &Secret) -> bool {
    let mut windows: HashSet<&[u8]> = HashSet::new();
    for shape in shapes(authorization) {
        if shape.len() < ECHO_WINDOW_BYTES {
            if hay.windows(shape.len()).any(|run| run == shape) {
                return true;
            }
        } else {
            windows.extend(shape.windows(ECHO_WINDOW_BYTES));
        }
    }
    hay.windows(ECHO_WINDOW_BYTES)
        .any(|run| windows.contains(run))
}

/// A bounded, printable excerpt of a registry's `body`: bytes outside printable ASCII become `?`
/// and white space a space, and a body over [`DETAIL_EXCERPT_BYTES`] ends in `...`.
fn bounded_excerpt(body: &[u8]) -> String {
    let mut excerpt: String = body
        .iter()
        .take(DETAIL_EXCERPT_BYTES)
        .map(|&b| match b {
            b'\n' | b'\r' | b'\t' => ' ',
            0x20..=0x7e => char::from(b),
            _ => '?',
        })
        .collect();
    if body.len() > DETAIL_EXCERPT_BYTES {
        excerpt.push_str("...");
    }
    excerpt
}

/// A bounded excerpt of a registry's `body` for a diagnostic, or [`WITHHELD_ECHO`] if the whole
/// body echoes the credential, including past the bound of the excerpt.
pub fn excerpt_unless_echoes(body: &[u8], authorization: &Secret) -> String {
    if echoes_credential(body, authorization) {
        WITHHELD_ECHO.to_owned()
    } else {
        bounded_excerpt(body)
    }
}

/// `detail`, or [`WITHHELD_ECHO`] if it echoes the credential. For text that is not a registry's
/// body, such as the message of a transport fault.
pub fn withhold_if_echoes(detail: String, authorization: &Secret) -> String {
    if echoes_credential(detail.as_bytes(), authorization) {
        WITHHELD_ECHO.to_owned()
    } else {
        detail
    }
}
