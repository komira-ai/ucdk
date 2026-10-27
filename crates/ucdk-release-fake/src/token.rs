// SPDX-License-Identifier: Apache-2.0
//! What the fake issuer writes into a grant and the fake registry reads back: the scope name,
//! the repository and the coordinates it covers. A grant's secret is only ever these bytes.
//!
//! Every text here is written as its length in bytes, a `:` and the text itself. No text can
//! then run into the next, whatever characters it holds, so two different lists of texts are
//! never the same bytes.

use ucdk_release_proto::v1::Coordinate;

/// Appends `text` as its length, a `:` and the text.
fn put(out: &mut String, text: &str) {
    out.push_str(&text.len().to_string());
    out.push(':');
    out.push_str(text);
}

/// The text a coordinate is kept under: its five fields in a fixed order, each with its length.
/// Two coordinates share a key only when every field is equal.
pub fn coordinate_key(c: &Coordinate) -> String {
    let mut key = String::new();
    for field in [
        c.channel.as_str(),
        &c.r#type.to_string(),
        &c.name,
        &c.version,
        &c.platform,
    ] {
        put(&mut key, field);
    }
    key
}

/// What a grant says it covers.
pub struct Claim {
    pub scope: String,
    pub repository: String,
    pub coordinates: Vec<String>,
}

pub fn encode(scope: &str, repository: &str, coordinates: &[Coordinate]) -> Vec<u8> {
    let mut secret = String::new();
    put(&mut secret, scope);
    put(&mut secret, repository);
    for c in coordinates {
        put(&mut secret, &coordinate_key(c));
    }
    secret.into_bytes()
}

/// The text at the start of `rest` and what follows it; none when `rest` does not start with a
/// length, a `:` and that many bytes.
fn take(rest: &[u8]) -> Option<(&[u8], &[u8])> {
    let colon = rest.iter().position(|b| *b == b':')?;
    let len: usize = std::str::from_utf8(&rest[..colon]).ok()?.parse().ok()?;
    let body = &rest[colon + 1..];
    (len <= body.len()).then(|| body.split_at(len))
}

/// Every text of `secret`, or none when it is not a run of whole texts.
fn texts(mut rest: &[u8]) -> Option<Vec<String>> {
    let mut all = Vec::new();
    while !rest.is_empty() {
        let (text, after) = take(rest)?;
        all.push(String::from_utf8_lossy(text).into_owned());
        rest = after;
    }
    Some(all)
}

/// Reads what [`encode`] wrote; bytes that are not that read as a claim that covers nothing.
pub fn decode(secret: &[u8]) -> Claim {
    let mut texts = texts(secret).unwrap_or_default().into_iter();
    Claim {
        scope: texts.next().unwrap_or_default(),
        repository: texts.next().unwrap_or_default(),
        coordinates: texts.collect(),
    }
}
