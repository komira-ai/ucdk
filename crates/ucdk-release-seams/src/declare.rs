// SPDX-License-Identifier: Apache-2.0
//! The declaration of the artifact names a release publishes, and the checks every publisher
//! applies to one before it keeps it.

use std::error::Error;
use std::fmt;

/// Why a declaration of names was refused. Each variant names the offending entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclareError {
    /// The entry at `index` is the empty string.
    Empty { index: usize },
    /// The name holds whitespace.
    Whitespace { name: String },
    /// The name holds a `/`.
    Slash { name: String },
    /// The name is an earlier entry again, ignoring ASCII case.
    Duplicate { name: String },
}

impl fmt::Display for DeclareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a declaration was refused")
    }
}

impl Error for DeclareError {}

/// Checks a declaration. Nothing is checked yet.
pub fn check_declaration(_names: &[String]) -> Result<(), DeclareError> {
    Ok(())
}
