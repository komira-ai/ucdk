// SPDX-License-Identifier: Apache-2.0
//! mopm's grammars, checked on the client with the same rules the server applies, so that a
//! request mopm would answer with a 400 is never sent (and so never retried, and never charged
//! against the daily file allowance).
//!
//! Nothing here normalises: an input outside a grammar is refused, never mapped to something
//! similar.

/// The longest version mopm stores.
pub const MAX_VERSION_LENGTH: usize = 128;
/// The longest file name mopm accepts.
pub const MAX_FILE_NAME_LENGTH: usize = 255;
/// The longest yank reason, in UTF-16 code units: mopm counts with JavaScript's `length`.
pub const MAX_REASON_UNITS: usize = 280;

/// `^[a-z][a-z0-9_]{1,63}$`.
pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (2..=64).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

/// Exactly `MAJOR.MINOR.PATCH`: three numbers, no leading zeros, nothing else, at most
/// [`MAX_VERSION_LENGTH`] characters.
pub fn valid_version(version: &str) -> bool {
    let number = |part: &str| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && (part == "0" || !part.starts_with('0'))
    };
    let parts: Vec<&str> = version.split('.').collect();
    version.len() <= MAX_VERSION_LENGTH && parts.len() == 3 && parts.into_iter().all(number)
}

/// `^mojo[0-9]+(?:\.[0-9]+)*_0$`.
pub fn valid_build(build: &str) -> bool {
    let Some(core) = build
        .strip_prefix("mojo")
        .and_then(|rest| rest.strip_suffix("_0"))
    else {
        return false;
    };
    core.split('.')
        .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// `^[A-Za-z0-9][A-Za-z0-9._+-]{0,254}$`.
pub fn safe_file_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=MAX_FILE_NAME_LENGTH).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

/// The build of `file`, if `file` is a safe file name of exactly the form
/// `mojo-<name>-<version>-<build>.conda` for this `name` and `version` and holds a valid build.
pub fn build_of_file<'a>(file: &'a str, name: &str, version: &str) -> Option<&'a str> {
    let prefix = format!("mojo-{name}-{version}-");
    let build = file.strip_prefix(prefix.as_str())?.strip_suffix(".conda")?;
    (safe_file_name(file) && valid_build(build)).then_some(build)
}

/// A yank reason: 1 to [`MAX_REASON_UNITS`] UTF-16 code units, so a character outside the basic
/// plane counts twice.
pub fn valid_reason(reason: &str) -> bool {
    (1..=MAX_REASON_UNITS).contains(&reason.encode_utf16().count())
}

/// A SHA-256 digest as mopm writes it: 64 lower-case hex digits.
pub fn valid_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
