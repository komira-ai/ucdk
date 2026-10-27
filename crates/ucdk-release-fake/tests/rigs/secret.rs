// SPDX-License-Identifier: Apache-2.0
//! The secret of a fake grant as a registry reads it and as a stub of an issuer writes it.
//!
//! The fake issuer writes every text of a secret as its length in bytes, a `:` and the text
//! itself, so a text may hold any character without running into the next. A rig reads the
//! texts of a secret with [`texts`], and builds a secret of its own with [`framed`]; neither
//! splits on a character, so a name that holds a newline or a `/` stays one text.

/// The text at the start of `rest` and what follows it; none when `rest` does not start with a
/// length, a `:` and that many bytes.
fn take(rest: &[u8]) -> Option<(&[u8], &[u8])> {
    let colon = rest.iter().position(|b| *b == b':')?;
    let len: usize = std::str::from_utf8(&rest[..colon]).ok()?.parse().ok()?;
    let body = &rest[colon + 1..];
    (len <= body.len()).then(|| body.split_at(len))
}

/// Every text of `secret`, in order; none at all when the secret is not a run of whole texts.
pub fn texts(secret: &[u8]) -> Vec<String> {
    let mut all = Vec::new();
    let mut rest = secret;
    while !rest.is_empty() {
        let Some((text, after)) = take(rest) else {
            return Vec::new();
        };
        all.push(String::from_utf8_lossy(text).into_owned());
        rest = after;
    }
    all
}

/// The secret that holds exactly `texts`, in order.
pub fn framed(texts: &[String]) -> Vec<u8> {
    let mut secret = Vec::new();
    for text in texts {
        secret.extend_from_slice(text.len().to_string().as_bytes());
        secret.push(b':');
        secret.extend_from_slice(text.as_bytes());
    }
    secret
}
