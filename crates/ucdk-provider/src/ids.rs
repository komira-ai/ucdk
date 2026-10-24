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
    /// Two fields of one node share a name.
    DuplicateField(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath => write!(f, "a node path is empty"),
            Self::BadSegment(s) => write!(f, "`{s}` is not a valid path segment"),
            Self::DuplicateField(n) => write!(f, "field `{n}` is given twice"),
        }
    }
}

impl std::error::Error for ModelError {}

fn valid_segment(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && s.len() <= 32
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The first path segment of a node: the top-level resource that owns it, readable on the
/// object.
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

/// The path of a node: `top/child/.../role`, any number of segments, each matching
/// `[a-z][a-z0-9-]{0,31}`. The first segment is the owner.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodePath(String);

impl NodePath {
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

    pub fn key(&self) -> NodeKey {
        NodeKey::of(self)
    }
}

impl fmt::Display for NodePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
/// The domain prefix of the node-key digest.
const KEY_DOMAIN: &[u8] = b"ucdk/node-key/1\n";
const KEY_LEN: usize = 26;

/// The stamped identity of a node: SHA-256 over a fixed domain prefix and the path, cut to
/// 128 bits and written as 26 lowercase base32 characters. A provider's label value is short,
/// so the path itself is never stamped.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeKey(String);

impl NodeKey {
    pub fn of(path: &NodePath) -> Self {
        let mut hash = Sha256::new();
        hash.update(KEY_DOMAIN);
        hash.update(path.as_str().as_bytes());
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

    /// Reads a key back from the text a provider stored: the shape is checked, nothing else.
    pub fn from_text(text: &str) -> Option<Self> {
        let shaped = text.len() == KEY_LEN && text.bytes().all(|b| ALPHABET.contains(&b));
        shaped.then(|| Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NodeKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The cloud's identifier for a live object. Opaque: equality and printing only.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PhysicalId(String);

impl PhysicalId {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PhysicalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A SHA-256 digest of the modelled fields of a node, in hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest(String);

impl Digest {
    pub(crate) fn from_bytes(bytes: &[u8]) -> Self {
        Self(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
