// SPDX-License-Identifier: Apache-2.0
//! The schema-version policy of a document ucdk reads.

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
    /// The document states no version (an unset field reads as 0).
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
    /// A policy that writes `current` and reads every major from `oldest_read` to `current`.
    pub const fn new(current: i32, oldest_read: i32) -> Self {
        Self {
            current,
            oldest_read,
        }
    }

    /// The major a writer states.
    pub const fn current(self) -> i32 {
        self.current
    }

    /// Whether a reader accepts a document that states `found`.
    pub fn check(self, found: i32) -> Result<(), SchemaRefusal> {
        let _ = found;
        Ok(())
    }
}

impl fmt::Display for SchemaRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("")
    }
}
