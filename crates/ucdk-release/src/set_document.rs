// SPDX-License-Identifier: Apache-2.0
//! The set document: the `ReleaseSet` a build stores beside its members. [`seal`] fills in the
//! schema version, sorts the members by name and writes the set hash, and [`encode`] gives the
//! protobuf bytes of that. It is not trusted: every reader recomputes the set hash from the
//! members and refuses a differing value, and a member's store key is never read from it.
//!
//! [`read`] accepts any protobuf bytes that decode to a document of the current schema version
//! whose members each carry a manifest, are sorted by name with each name once, and whose stored
//! hash is the hash of the members. It does not require the bytes to be what [`encode`] writes
//! (unknown fields are dropped and fields may come in any order), so the bytes of a document
//! are not an identity: compare set hashes, never documents. It does not compare the revision
//! or the platform of the set with any scope; the caller does.

use std::fmt;

use prost::Message;
use ucdk_release_proto::v1::{Member, Reason, ReleaseSet};

use crate::hashes::set_hash;

/// The schema version of the set document.
pub const SET_SCHEMA: i32 = 1;

/// Why a set document was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetError {
    /// The bytes are not a set document.
    Decode(String),
    /// The document is of another schema version.
    Version(i32),
    /// A member carries no manifest.
    MemberWithoutManifest(usize),
    /// The members are not sorted by name, each name once.
    MembersNotSorted(String),
    /// The stored hash is not the hash of the members.
    Hash { stored: String, computed: String },
}

impl SetError {
    /// The reason a verdict carries: a document that is missing, unreadable or disagrees with
    /// its own members is a set hash fault; a malformed member list is a member fault.
    pub fn reason(&self) -> Reason {
        match self {
            SetError::MemberWithoutManifest(_) | SetError::MembersNotSorted(_) => Reason::SetMember,
            SetError::Decode(_) | SetError::Version(_) | SetError::Hash { .. } => Reason::SetHash,
        }
    }
}

impl fmt::Display for SetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetError::Decode(why) => write!(f, "the set document does not decode: {why}"),
            SetError::Version(found) => write!(
                f,
                "the set document is schema version {found}, and this reader knows {SET_SCHEMA}"
            ),
            SetError::MemberWithoutManifest(at) => {
                write!(f, "member {at} of the set has no manifest")
            }
            SetError::MembersNotSorted(name) => write!(
                f,
                "the members are not sorted by name, each name once, at {name:?}"
            ),
            SetError::Hash { stored, computed } => write!(
                f,
                "the set document says its hash is {stored:?}, and its members hash to {computed}"
            ),
        }
    }
}

fn name_of(member: &Member) -> &str {
    member.manifest.as_ref().map_or("", |m| m.name.as_str())
}

/// The set ready to be written: schema version set, members sorted by name, and the hash of
/// its members in `set_hash`.
pub fn seal(mut set: ReleaseSet) -> ReleaseSet {
    set.schema_version = SET_SCHEMA;
    set.members.sort_by(|a, b| name_of(a).cmp(name_of(b)));
    set.set_hash = set_hash(&set).to_string();
    set
}

/// The canonical bytes of a sealed set.
pub fn encode(set: &ReleaseSet) -> Vec<u8> {
    set.encode_to_vec()
}

/// Reads a set document and checks what a reader must: the version, that every member has a
/// manifest and the members are sorted, and that the stored hash is the hash of the members.
pub fn read(bytes: &[u8]) -> Result<ReleaseSet, SetError> {
    let set = ReleaseSet::decode(bytes).map_err(|e| SetError::Decode(e.to_string()))?;
    if set.schema_version != SET_SCHEMA {
        return Err(SetError::Version(set.schema_version));
    }
    if let Some(at) = set.members.iter().position(|m| m.manifest.is_none()) {
        return Err(SetError::MemberWithoutManifest(at));
    }
    if let Some(pair) = set
        .members
        .windows(2)
        .find(|w| name_of(&w[0]) >= name_of(&w[1]))
    {
        return Err(SetError::MembersNotSorted(name_of(&pair[1]).to_owned()));
    }
    let computed = set_hash(&set).to_string();
    if computed == set.set_hash {
        Ok(set)
    } else {
        Err(SetError::Hash {
            stored: set.set_hash,
            computed,
        })
    }
}
