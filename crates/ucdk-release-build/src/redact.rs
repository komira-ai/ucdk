// SPDX-License-Identifier: Apache-2.0
//! Redaction of the output of a tool before it is printed, logged or put in a report.
//!
//! This is a net under the rule that matters, which is not to put a credential in a message at
//! all. It replaces the value of a secret name, and the userinfo of a URL:
//!
//! - a name from [`KEYS`], matched in any letter case and as a part of a longer name
//!   (`access_token`, `X-Amz-Signature`; `sig` and `auth` only as whole words), followed by
//!   - `=`, `:` or their percent-encoded forms, with spaces and tabs around it, as in a query
//!     string, a header or a JSON member (the name may be quoted);
//!   - or, as a command-line flag (`--token <value>`), by spaces or tabs;
//! - `Bearer <value>`;
//! - an `Authorization` header with or without a scheme (`Basic`, `Digest`, `Bearer`);
//! - the userinfo of a URL: `scheme://<userinfo>@host`.
//!
//! A value that opens with a quote runs to the matching quote, and a backslash escapes the next
//! byte. Any other value ends at white space, a quote, `&`, `,`, `;` or `}`. A name with nothing
//! after it on its line replaces nothing.
//!
//! Known limits, each pinned by a test:
//!
//! - The net is wide on prose: `the bearer of this message` and `password: required` lose the
//!   word after the name.
//! - A JSON string that is itself escaped inside another string (`{\"token\":\"x\"}`) is not
//!   recognised.
//! - The authority of a URL ends at `/`, `?` or `#`, so a userinfo password that holds one of
//!   them unencoded is not recognised. Encoded (`%2F`) it is.
//! - A secret under another name, or one printed bare, is not recognised.

/// What replaces a credential-shaped run. Deliberately loud: a redaction that cannot be seen is a
/// diagnosis made wrongly.
pub const REDACTED: &str = "[REDACTED]";

/// A name whose value is a secret.
struct Key {
    name: &'static str,
    /// Matches only at the start of a word, so that `design=` is not `sig=`.
    whole_word: bool,
    /// The value may be preceded by an authentication scheme, which is kept.
    scheme: bool,
    /// The value follows the name after spaces or tabs alone, as `Bearer <value>` does.
    bare: bool,
}

const fn key(name: &'static str) -> Key {
    Key {
        name,
        whole_word: false,
        scheme: false,
        bare: false,
    }
}

const fn word(name: &'static str) -> Key {
    Key {
        whole_word: true,
        ..key(name)
    }
}

const KEYS: [Key; 13] = [
    Key {
        bare: true,
        ..key("bearer")
    },
    Key {
        scheme: true,
        ..key("authorization")
    },
    key("token"),
    key("password"),
    key("passwd"),
    key("secret"),
    key("api_key"),
    key("api-key"),
    key("apikey"),
    key("signature"),
    key("credential"),
    word("sig"),
    word("auth"),
];

/// `text` with the credential-shaped runs replaced by [`REDACTED`].
pub fn redact(text: &str) -> String {
    let mut out = text.to_owned();
    for key in &KEYS {
        out = redact_key(&out, key);
    }
    redact_userinfo(&out)
}

/// Replaces the value of each occurrence of `key` (see [`value_span`]) with [`REDACTED`], unless
/// it is empty. Every cut is at an ASCII byte, so the text stays valid.
fn redact_key(text: &str, key: &Key) -> String {
    let (bytes, name) = (text.as_bytes(), key.name.as_bytes());
    let mut out = String::with_capacity(text.len());
    let (mut at, mut copied) = (0, 0);
    while at + name.len() <= bytes.len() {
        let named = bytes[at..at + name.len()].eq_ignore_ascii_case(name);
        let span = if named {
            value_span(bytes, at, key)
        } else {
            None
        };
        let Some((start, end)) = span else {
            at += 1;
            continue;
        };
        out.push_str(&text[copied..start]);
        if end > start {
            out.push_str(REDACTED);
        }
        (at, copied) = (end, end);
    }
    out.push_str(&text[copied..]);
    out
}

/// The span of the value of the `key` that stands at `at`, if what follows the name is a value.
fn value_span(bytes: &[u8], at: usize, key: &Key) -> Option<(usize, usize)> {
    if key.whole_word && at > 0 && bytes[at - 1].is_ascii_alphanumeric() {
        return None;
    }
    let name_end = at + key.name.len();
    let quoted_name = usize::from(matches!(bytes.get(name_end), Some(b'"' | b'\'')));
    let blanks = skip_blanks(bytes, name_end + quoted_name);
    let start = match separator(&bytes[blanks..]) {
        Some(len) => skip_blanks(bytes, blanks + len),
        None if blanks > name_end
            && bytes.get(blanks) != Some(&b'-')
            && (key.bare || in_flag(bytes, at)) =>
        {
            blanks
        }
        None => return None,
    };
    let (first, end) = run(bytes, start);
    if key.scheme
        && SCHEMES
            .iter()
            .any(|s| bytes[first..end].eq_ignore_ascii_case(s))
    {
        let next = skip_blanks(bytes, end);
        if next > end {
            return Some(run(bytes, next));
        }
    }
    Some((first, end))
}

/// The authentication schemes that may stand before a credential.
const SCHEMES: [&[u8]; 3] = [b"bearer", b"basic", b"digest"];

fn skip_blanks(bytes: &[u8], from: usize) -> usize {
    from + bytes[from..]
        .iter()
        .take_while(|&&b| b == b' ' || b == b'\t')
        .count()
}

/// The length of the separator at the start of `rest`: `=` or `:`, or either percent-encoded.
fn separator(rest: &[u8]) -> Option<usize> {
    if matches!(rest.first(), Some(b'=' | b':')) {
        return Some(1);
    }
    let encoded = rest.get(..3)?;
    (encoded.eq_ignore_ascii_case(b"%3d") || encoded.eq_ignore_ascii_case(b"%3a")).then_some(3)
}

/// Whether the name at `at` is the end of a command-line flag: `--token`, `--access-token`.
fn in_flag(bytes: &[u8], at: usize) -> bool {
    let word = bytes[..at]
        .iter()
        .rev()
        .take_while(|&&b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        .count();
    bytes[at - word..at].starts_with(b"--")
}

/// The run that starts at `start`: to the matching quote if it opens with one, else to the first
/// byte that ends an unquoted value. Returns where the replaced part starts and ends.
fn run(bytes: &[u8], start: usize) -> (usize, usize) {
    match bytes.get(start) {
        Some(&quote @ (b'"' | b'\'')) => (start + 1, closing(bytes, start + 1, quote)),
        _ => {
            let len = bytes[start..].iter().position(|&b| ends_value(b));
            (start, len.map_or(bytes.len(), |n| start + n))
        }
    }
}

fn ends_value(byte: u8) -> bool {
    byte.is_ascii_whitespace() || matches!(byte, b'&' | b'"' | b'\'' | b',' | b';' | b'}')
}

/// Where the string that opened with `quote` ends: its closing quote, with a backslash escaping
/// the byte after it, or the end of `bytes`.
fn closing(bytes: &[u8], from: usize, quote: u8) -> usize {
    let mut at = from;
    while at < bytes.len() && bytes[at] != quote {
        at += if bytes[at] == b'\\' { 2 } else { 1 };
    }
    at.min(bytes.len())
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
