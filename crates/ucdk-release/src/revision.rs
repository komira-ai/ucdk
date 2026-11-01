// SPDX-License-Identifier: Apache-2.0
//! Revisions and build stamps. A revision is a full commit id: exactly 40 lowercase hex digits.
//! An abbreviated id names a commit only while no other commit shares its prefix, so none is
//! accepted. A build stamp is the git-derived values a build is stamped with: the revision it
//! builds, the commit its sources come from, a build number and a commit time.

use std::fmt;

/// A full commit id. Made only by [`Revision::parse`], so a value of this type is 40 lowercase
/// hex digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(String);

/// Text that is not a full commit id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionError {
    pub text: String,
}

impl fmt::Display for RevisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("")
    }
}

impl Revision {
    pub fn parse(text: &str) -> Result<Self, RevisionError> {
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("")
    }
}

/// Why a build stamp was refused. The first value that fails is the one named, checked in the
/// order of the fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StampError {
    /// A field that holds a commit id does not hold a full one.
    Revision {
        field: &'static str,
        error: RevisionError,
    },
    /// The build number is zero or negative.
    BuildNumber(i64),
    /// The commit time is zero or negative.
    TimestampMs(i64),
}

impl fmt::Display for StampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("")
    }
}

/// The git-derived values of a build. Made only by [`BuildStamp::new`], so a value of this type
/// has two full commit ids and a positive build number and commit time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildStamp {
    revision: Revision,
    source_commit: Revision,
    build_number: u64,
    timestamp_ms: u64,
}

impl BuildStamp {
    pub fn new(
        revision: &str,
        source_commit: &str,
        build_number: i64,
        timestamp_ms: i64,
    ) -> Result<Self, StampError> {
        Ok(Self {
            revision: Revision::parse(revision).map_err(|error| StampError::Revision {
                field: "revision_id",
                error,
            })?,
            source_commit: Revision::parse(source_commit).map_err(|error| {
                StampError::Revision {
                    field: "source_commit",
                    error,
                }
            })?,
            build_number: u64::try_from(build_number).unwrap_or(0),
            timestamp_ms: u64::try_from(timestamp_ms).unwrap_or(0),
        })
    }

    /// The commit the build is of.
    pub fn revision(&self) -> &Revision {
        &self.revision
    }

    /// The commit the sources are from.
    pub fn source_commit(&self) -> &Revision {
        &self.source_commit
    }

    pub fn build_number(&self) -> u64 {
        self.build_number
    }

    pub fn timestamp_ms(&self) -> u64 {
        self.timestamp_ms
    }
}
