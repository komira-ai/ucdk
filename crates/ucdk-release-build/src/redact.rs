// SPDX-License-Identifier: Apache-2.0
//! Redaction of the output of a tool before it is printed, logged or put in a report.
//!
//! This is a net under the rule that matters, which is not to put a credential in a message at
//! all. It replaces four shapes that build tools and HTTP clients print:
//!
//! - `Bearer <run>`;
//! - `<key>=<run>`, as in a query string or a command line;
//! - `<key>":"<run>` and `<key>": "<run>`, as in a JSON object;
//! - the userinfo of a URL: `scheme://<userinfo>@host`.
//!
//! The keys are in [`KEYS`] and match in any letter case, as a part of a longer name too
//! (`access_token`, `X-Amz-Signature`). A run ends at white space, a quote or (after `=`) `&`. A
//! JSON string that is itself escaped inside another string is not recognised. The set is narrow
//! on purpose: a net that mangles ordinary text makes people stop reading the output.

/// What replaces a credential-shaped run. Deliberately loud: a redaction that cannot be seen is a
/// diagnosis made wrongly.
pub const REDACTED: &str = "[REDACTED]";

/// The names whose value is a secret.
const KEYS: [&str; 8] = [
    "token",
    "password",
    "passwd",
    "secret",
    "api_key",
    "apikey",
    "signature",
    "credential",
];

/// `text` with the credential-shaped runs replaced by [`REDACTED`].
pub fn redact(text: &str) -> String {
    let mut out = redact_after(text, "bearer ", ends_assignment);
    for key in KEYS {
        out = redact_after(&out, &format!("{key}="), ends_assignment);
        out = redact_after(&out, &format!("{key}\":\""), ends_string);
        out = redact_after(&out, &format!("{key}\": \""), ends_string);
    }
    redact_userinfo(&out)
}

fn ends_assignment(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'&' | b'"' | b'\'')
}

fn ends_string(byte: u8) -> bool {
    byte == b'"'
}

/// Replaces the run that follows each `marker` (compared without regard to letter case) with
/// [`REDACTED`], unless the run is empty. Every cut is at an ASCII byte, so the text stays valid.
fn redact_after(text: &str, marker: &str, ends: fn(u8) -> bool) -> String {
    let (bytes, marker) = (text.as_bytes(), marker.as_bytes());
    let mut out = String::with_capacity(text.len());
    let (mut at, mut copied) = (0, 0);
    while at + marker.len() <= bytes.len() {
        if !bytes[at..at + marker.len()].eq_ignore_ascii_case(marker) {
            at += 1;
            continue;
        }
        let start = at + marker.len();
        let end = bytes[start..]
            .iter()
            .position(|&b| ends(b))
            .map_or(bytes.len(), |n| start + n);
        out.push_str(&text[copied..start]);
        if end > start {
            out.push_str(REDACTED);
        }
        (at, copied) = (end, end);
    }
    out.push_str(&text[copied..]);
    out
}

/// Replaces the userinfo of every URL: what stands between `://` and the last `@` of the
/// authority, which ends at `/`, `?`, `#`, a quote or white space.
fn redact_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(scheme_end) = rest.find("://") {
        let (head, tail) = rest.split_at(scheme_end + 3);
        out.push_str(head);
        let authority_end = tail
            .find(|c: char| matches!(c, '/' | '?' | '#' | '"' | '\'') || c.is_whitespace())
            .unwrap_or(tail.len());
        rest = tail;
        if let Some(at) = tail[..authority_end].rfind('@') {
            out.push_str(REDACTED);
            rest = &tail[at..];
        }
    }
    out.push_str(rest);
    out
}
