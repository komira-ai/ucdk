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
        match self {
            Self::Empty { index } => write!(f, "declared name {index} is empty"),
            Self::Whitespace { name } => write!(f, "declared name {name:?} holds whitespace"),
            Self::Slash { name } => write!(f, "declared name {name:?} holds a '/'"),
            Self::Duplicate { name } => write!(
                f,
                "declared name {name:?} repeats an earlier one, ignoring ASCII case"
            ),
        }
    }
}

impl Error for DeclareError {}

/// Checks a declaration, entry by entry in order, and answers the first entry that is refused:
/// an empty name, a name that holds whitespace (any `char::is_whitespace`, asked before the
/// slash), a name that holds a `/`, or a name that is an earlier entry again, ignoring ASCII
/// case. Nothing else is done to a name: it is not trimmed, non-ASCII letters are not
/// case-folded (`\u{c9}` and `\u{e9}` are two names) and no registry's own normalisation is
/// applied (`my_pkg` and `my-pkg` are two names). An empty list is accepted and declares
/// nothing.
pub fn check_declaration(names: &[String]) -> Result<(), DeclareError> {
    for (index, name) in names.iter().enumerate() {
        if name.is_empty() {
            return Err(DeclareError::Empty { index });
        }
        if name.chars().any(char::is_whitespace) {
            return Err(DeclareError::Whitespace { name: name.clone() });
        }
        if name.contains('/') {
            return Err(DeclareError::Slash { name: name.clone() });
        }
        if names[..index].iter().any(|n| n.eq_ignore_ascii_case(name)) {
            return Err(DeclareError::Duplicate { name: name.clone() });
        }
    }
    Ok(())
}
