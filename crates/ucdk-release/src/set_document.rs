// SPDX-License-Identifier: Apache-2.0
//! The set document: the `ReleaseSet` a build stores beside its members, in canonical
//! encoding. It is not trusted: every reader recomputes the set hash from the members and
//! refuses a differing value, and a member's store key is never read from it.

use std::fmt;

use prost::Message;
use ucdk_release_proto::v1::{Reason, ReleaseSet};

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
            SetError::Version(found) => {
                write!(
                    f,
                    "the set document is schema version {found}, and this reader knows {SET_SCHEMA}"
                )
            }
            SetError::MemberWithoutManifest(at) => {
                write!(f, "member {at} of the set has no manifest")
            }
            SetError::MembersNotSorted(name) => {
                write!(
                    f,
                    "the members are not sorted by name, each name once, at {name:?}"
                )
            }
            SetError::Hash { stored, computed } => {
                write!(
                    f,
                    "the set document says its hash is {stored:?}, and its members hash to {computed}"
                )
            }
        }
    }
}

/// The set ready to be written: schema version set, members sorted by name, and the hash of
/// its members in `set_hash`.
pub fn seal(mut set: ReleaseSet) -> ReleaseSet {
    set.schema_version = SET_SCHEMA;
    set.members.sort_by(|a, b| {
        let name = |m: &ucdk_release_proto::v1::Member| m.manifest.clone().unwrap_or_default().name;
        name(a).cmp(&name(b))
    });
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
    let mut names = Vec::new();
    for (at, member) in set.members.iter().enumerate() {
        let manifest = member
            .manifest
            .as_ref()
            .ok_or(SetError::MemberWithoutManifest(at))?;
        names.push(manifest.name.as_str());
    }
    if let Some(pair) = names.windows(2).find(|w| w[0] >= w[1]) {
        return Err(SetError::MembersNotSorted(pair[1].to_owned()));
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
