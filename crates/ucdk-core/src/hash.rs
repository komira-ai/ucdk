// SPDX-License-Identifier: Apache-2.0
//! A 256-bit hash and the length-framed way to feed one.

use std::fmt;

use sha2::{Digest as _, Sha256};

/// A SHA-256 hash, as 64 lower-case hex digits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hash256(String);

impl Hash256 {
    /// The hash of a finished SHA-256 computation.
    pub fn from_sha256(digest: &[u8; 32]) -> Self {
        Self(digest.iter().map(|b| format!("{b:02x}")).collect())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Reads a hash from its text: 64 lower-case hex digits, as [`Hash256::as_str`] writes it.
    /// This is how a hash pinned by an approver comes back from a flag or a file.
    pub fn from_hex(text: &str) -> Option<Self> {
        let well_formed = text.len() == 64
            && text
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        well_formed.then(|| Self(text.to_owned()))
    }
}

impl fmt::Display for Hash256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A hash over a domain tag and a list of fields, each framed by its length so that two lists
/// that concatenate to the same bytes still hash differently.
///
/// The domain bytes are fed as they are, unframed: a caller passes its tag with the trailing
/// newline (`b"ucdk/example/1\n"`). Each field is fed as its length in eight big-endian bytes
/// and then its bytes.
#[derive(Debug, Clone)]
pub struct Framed(Sha256);

impl Framed {
    pub fn new(domain: &[u8]) -> Self {
        let mut hash = Sha256::new();
        hash.update(domain);
        Self(hash)
    }

    /// Feeds one field.
    #[must_use]
    pub fn field(mut self, bytes: &[u8]) -> Self {
        self.0.update((bytes.len() as u64).to_be_bytes());
        self.0.update(bytes);
        self
    }

    pub fn finish(self) -> Hash256 {
        Hash256::from_sha256(&self.0.finalize().into())
    }
}
