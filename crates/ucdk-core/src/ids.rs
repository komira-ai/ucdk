// SPDX-License-Identifier: Apache-2.0
//! Validated identifiers. Each is compared and printed, never parsed for meaning.

use std::fmt;

use sha2::{Digest as _, Sha256};

/// A model value that is not well formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    EmptyPath,
    /// A path segment that does not match `[a-z][a-z0-9-]{0,31}`.
    BadSegment(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => write!(f, "a record path is empty"),
            Self::BadSegment(s) => write!(f, "`{s}` is not a valid path segment"),
        }
    }
}

impl std::error::Error for ModelError {}

/// The longest path segment or owner label, in bytes. It reaches the ledger file, so it is a
/// format promise: a longer limit later is a compatible change, a shorter one is not.
pub const SEGMENT_MAX: usize = 32;

pub(crate) fn valid_segment(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && s.len() <= SEGMENT_MAX
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The first path segment of a record: the top-level owner, readable in the ledger.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct OwnerLabel(String);

impl OwnerLabel {
    pub fn new(text: &str) -> Result<Self, ModelError> {
        if valid_segment(text) {
            Ok(Self(text.to_owned()))
        } else {
            Err(ModelError::BadSegment(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OwnerLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The path of a record: `top/child/.../leaf`, any number of segments, each matching
/// `[a-z][a-z0-9-]{0,31}`. The first segment is the owner. It is named `RecordPath` so that
/// nothing is called `Path` next to `std::path::Path`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RecordPath(String);

impl RecordPath {
    pub fn new(text: &str) -> Result<Self, ModelError> {
        if text.is_empty() {
            return Err(ModelError::EmptyPath);
        }
        match text.split('/').find(|s| !valid_segment(s)) {
            Some(bad) => Err(ModelError::BadSegment(bad.to_owned())),
            None => Ok(Self(text.to_owned())),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The owner: the first segment.
    pub fn owner(&self) -> OwnerLabel {
        let first = self.0.split('/').next().unwrap_or_default();
        OwnerLabel(first.to_owned())
    }

    pub fn key(&self) -> RecordKey {
        RecordKey::of(self)
    }
}

impl fmt::Display for RecordPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
/// The domain prefix of the key of a record path. A format promise: a ledger written before
/// the rename of `NodeKey` to [`RecordKey`] hashes its paths to the same keys.
const PATH_KEY_DOMAIN: &[u8] = b"ucdk/node-key/1\n";
/// The domain prefix of the key of a readable name that is not a path.
const NAME_KEY_DOMAIN: &[u8] = b"ucdk/record-key/1\n";
/// The length of a record key's text, in characters. A format promise, like [`SEGMENT_MAX`].
pub const NODE_KEY_LEN: usize = 26;

/// The identity of a record: SHA-256 over a domain prefix and the path (or the name), cut to
/// 128 bits and written as 26 lowercase base32 characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RecordKey(String);

impl RecordKey {
    /// The key of a path. Its domain is `ucdk/node-key/1` and does not change.
    pub fn of(path: &RecordPath) -> Self {
        Self::hashed(PATH_KEY_DOMAIN, path.as_str())
    }

    /// The key of a readable name that is not a path, such as a coordinate with a version. Its
    /// domain is `ucdk/record-key/1`, so a name and a path spelled alike never share a key.
    pub fn of_name(name: &str) -> Self {
        Self::hashed(NAME_KEY_DOMAIN, name)
    }

    fn hashed(domain: &[u8], text: &str) -> Self {
        let mut hash = Sha256::new();
        hash.update(domain);
        hash.update(text.as_bytes());
        let digest = hash.finalize();
        let mut head = [0u8; 16];
        head.copy_from_slice(&digest[..16]);
        let n = u128::from_be_bytes(head);
        // 25 full groups of five bits, then the last three bits padded with two zero bits.
        let mut text: String = (0..25)
            .map(|i| char::from(ALPHABET[((n >> (123 - 5 * i)) & 31) as usize]))
            .collect();
        text.push(char::from(ALPHABET[((n & 7) << 2) as usize]));
        Self(text)
    }

    /// Reads a key back from its text: the shape is checked, nothing else.
    pub fn from_text(text: &str) -> Option<Self> {
        let well_formed = text.len() == NODE_KEY_LEN && text.bytes().all(|b| ALPHABET.contains(&b));
        well_formed.then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The key as one path segment: `k` and the key (27 bytes). A key may start with a digit
    /// and a segment may not, so the letter is what makes it a segment.
    pub fn segment(&self) -> String {
        format!("k{}", self.0)
    }
}

impl fmt::Display for RecordKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What kind of thing a record is, as the record's writer names it. It is not validated: any
/// text is a tag, exactly as any text was a kind, so no ledger written earlier fails to read.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Tag(String);

impl Tag {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where the thing a record stands for is, as its writer names it. Opaque: equality and
/// printing only.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Locator(String);

impl Locator {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Locator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
