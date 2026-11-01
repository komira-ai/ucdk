// SPDX-License-Identifier: Apache-2.0
//! What "the same bytes" means when one side is us and the other is a registry.
//!
//! Our identity for a file is the sha256 of the bytes we send. A registry exposes whatever
//! digests it chooses: some a sha256, some only a sha512 (as an SRI string) and a sha1. A
//! [`ContentIdentity`] carries the three, and a field a side does not expose is `None`.
//!
//! Two identities are compared field by field, never across fields: a sha512 is never compared
//! with a sha256. An empty field is "not exposed", never "equal to anything". Two identities that
//! share no exposed field are [`IdentityMatch::NoCommonField`], an answer of its own that is not
//! agreement and that a caller refuses. Comparing only the fields both sides expose and calling an
//! empty intersection "no difference found" would let a registry that exposes nothing we compute
//! read as holding our bytes for ever.
//!
//! The hex fields (sha256 and sha1) are read without regard to letter case: they are stored in
//! lower case, so `ABC...` and `abc...` are the same digest. The sha512 field is an SRI string,
//! compared whole, byte for byte and as opaque text, because base64 is case-sensitive: a
//! multi-hash value, a `sha384-` prefix, padding or malformed text is not parsed, and the same
//! text with another letter case is another value.
//!
//! Only the empty string means "not exposed", so a value of a single space is exposed and differs
//! from any real digest. A sha1 shared on its own is a [`IdentityMatch::Match`]: this module sets
//! no policy against a weak hash, and refusing one would be a change of design.

/// The digests one side exposes for a file. A field that is `None` is not exposed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentIdentity {
    sha256: Option<String>,
    sha512: Option<String>,
    sha1: Option<String>,
}

impl ContentIdentity {
    /// An identity that exposes nothing: compared with any identity it is
    /// [`IdentityMatch::NoCommonField`].
    pub fn none() -> Self {
        Self::default()
    }

    /// An identity that exposes only a sha256. An empty `hex` is a digest that is not exposed, so
    /// it gives [`ContentIdentity::none`].
    pub fn of_sha256(hex: &str) -> Self {
        Self::none().with_sha256(hex)
    }

    /// This identity with its sha256 set to `hex` (kept in lower case), or not exposed if `hex` is
    /// empty.
    #[must_use]
    pub fn with_sha256(mut self, hex: &str) -> Self {
        self.sha256 = exposed(&hex.to_ascii_lowercase());
        self
    }

    /// This identity with its sha512 (`sha512-<base64>`) set to `sri`, or not exposed if `sri` is
    /// empty.
    #[must_use]
    pub fn with_sha512(mut self, sri: &str) -> Self {
        self.sha512 = exposed(sri);
        self
    }

    /// This identity with its sha1 set to `hex` (kept in lower case), or not exposed if `hex` is
    /// empty.
    #[must_use]
    pub fn with_sha1(mut self, hex: &str) -> Self {
        self.sha1 = exposed(&hex.to_ascii_lowercase());
        self
    }

    /// Whether no field is exposed.
    pub fn exposes_nothing(&self) -> bool {
        self.sha256.is_none() && self.sha512.is_none() && self.sha1.is_none()
    }
}

fn exposed(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

/// The three answers of [`identity_matches`]. Only `Match` is agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityMatch {
    /// At least one field is exposed on both sides, and every such field is equal.
    Match,
    /// A field exposed on both sides differs.
    Mismatch,
    /// No field is exposed on both sides. Never equality: a caller refuses it.
    NoCommonField,
}

/// Compares two identities field by field. A field exposed on one side only is ignored: it is a
/// witness one side holds beside the other's, not a difference.
pub fn identity_matches(ours: &ContentIdentity, theirs: &ContentIdentity) -> IdentityMatch {
    let fields = [
        (&ours.sha256, &theirs.sha256),
        (&ours.sha512, &theirs.sha512),
        (&ours.sha1, &theirs.sha1),
    ];
    let mut compared = false;
    for (a, b) in fields {
        if let (Some(a), Some(b)) = (a, b) {
            if a != b {
                return IdentityMatch::Mismatch;
            }
            compared = true;
        }
    }
    if compared {
        IdentityMatch::Match
    } else {
        IdentityMatch::NoCommonField
    }
}
