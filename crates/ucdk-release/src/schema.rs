// SPDX-License-Identifier: Apache-2.0
//! The schema-version policy of a document ucdk reads.
//!
//! A document states one integer, its major. A writer only adds inside a major: a new field, or
//! a new value that an old reader can ignore. Removing or renaming a field, changing what a
//! value means, or changing a default is a new major. So a reader accepts every major from the
//! oldest it still reads up to the current one, and ignores what it does not know inside them;
//! it refuses a newer major as one that needs a newer reader, and an older one as no longer
//! read. There is no minor to compare.

use std::fmt;

/// The majors a reader accepts of one kind of document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaPolicy {
    current: i32,
    oldest_read: i32,
}

/// Why a reader refuses the schema version of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaRefusal {
    /// The document states no version (an unset field reads as 0), or a version below 1.
    Missing { current: i32 },
    /// The document is of a newer major than this reader knows.
    NeedsNewer { found: i32, current: i32 },
    /// The document is of a major this reader no longer reads.
    NoLongerRead {
        found: i32,
        oldest_read: i32,
        current: i32,
    },
}

impl SchemaPolicy {
    /// A policy that writes `current` and reads every major from `oldest_read` to `current`;
    /// refused unless `1 <= oldest_read <= current`.
    pub fn new(current: i32, oldest_read: i32) -> Result<Self, PolicyError> {
        if !(1..=current).contains(&oldest_read) {
            return Err(PolicyError {
                current,
                oldest_read,
            });
        }
        Ok(Self {
            current,
            oldest_read,
        })
    }

    /// The major a writer states.
    pub const fn current(self) -> i32 {
        self.current
    }

    /// Whether a reader accepts a document that states `found`.
    pub fn check(self, found: i32) -> Result<(), SchemaRefusal> {
        if found < 1 {
            Err(SchemaRefusal::Missing {
                current: self.current,
            })
        } else if found > self.current {
            Err(SchemaRefusal::NeedsNewer {
                found,
                current: self.current,
            })
        } else if found < self.oldest_read {
            Err(SchemaRefusal::NoLongerRead {
                found,
                oldest_read: self.oldest_read,
                current: self.current,
            })
        } else {
            Ok(())
        }
    }
}

impl fmt::Display for SchemaRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            SchemaRefusal::Missing { current } => {
                write!(f, "no schema_version; add `schema_version: {current}`")
            }
            SchemaRefusal::NeedsNewer { found, current } => write!(
                f,
                "schema_version {found} needs a newer ucdk (this ucdk reads up to major {current})"
            ),
            SchemaRefusal::NoLongerRead {
                found,
                oldest_read,
                current,
            } if oldest_read == current => write!(
                f,
                "schema_version {found} is no longer read (this ucdk reads major {current})"
            ),
            SchemaRefusal::NoLongerRead {
                found,
                oldest_read,
                current,
            } => write!(
                f,
                "schema_version {found} is no longer read (this ucdk reads major {oldest_read}..{current})"
            ),
        }
    }
}

/// A range of majors that is not `1 <= oldest_read <= current`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyError {
    pub current: i32,
    pub oldest_read: i32,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "a policy needs 1 <= oldest read <= current, and this one has oldest read {} and current {}",
            self.oldest_read, self.current
        )
    }
}
