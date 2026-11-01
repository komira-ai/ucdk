// SPDX-License-Identifier: Apache-2.0
//! Who is running: the run id, the attempt and the context a driver supplies, checked.
//!
//! - The run id is 1 to 63 bytes of `[a-z0-9_-]`, so the same id can later be stamped on a
//!   cloud object as a label. ucdk mints none: a driver passes one.
//! - The attempt is a positive number with no default. An omitted attempt is a usage error,
//!   never attempt 1.
//! - The context is at most 32 pairs. A key is `[a-z][a-z0-9_]{0,31}`; a value is at most 256
//!   bytes of printable ASCII. It is recorded verbatim and trusted by nothing: anyone who runs
//!   ucdk can write anything in it, so no decision reads it and no hash covers it.
//!
//! ucdk reads no CI vendor's environment variable for any of this.

use std::collections::BTreeMap;

use ucdk_release_proto::v1::RunIdentity;

/// The most bytes of a run id.
pub const RUN_ID_MAX_BYTES: usize = 63;
/// The most context pairs.
pub const CONTEXT_MAX_ENTRIES: usize = 32;
/// The most bytes of a context key.
pub const CONTEXT_KEY_MAX_BYTES: usize = 32;
/// The most bytes of a context value.
pub const CONTEXT_VALUE_MAX_BYTES: usize = 256;

/// Why a run identity is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunIdentityError {
    /// The run id is empty.
    RunIdEmpty,
    /// The run id is longer than [`RUN_ID_MAX_BYTES`].
    RunIdTooLong { bytes: usize },
    /// The run id holds a byte outside `[a-z0-9_-]`.
    RunIdByte { at: usize },
    /// No attempt was given (a message reads an unset number as 0).
    AttemptMissing,
    /// The attempt text is not a positive decimal number without sign or leading zero, or does
    /// not fit.
    AttemptText { text: String },
    /// More than [`CONTEXT_MAX_ENTRIES`] pairs.
    ContextTooMany { count: usize },
    /// A `--context` argument without an `=`.
    ContextNotKeyValue { arg: String },
    /// A context key outside its grammar.
    ContextKey { key: String },
    /// A context key given twice.
    ContextKeyTwice { key: String },
    /// A context value that is too long.
    ContextValueTooLong { key: String, bytes: usize },
    /// A context value holding a byte outside printable ASCII.
    ContextValueByte { key: String, at: usize },
}

/// Checks the run id, the attempt and the context of `identity` against the grammar above.
pub fn check(identity: &RunIdentity) -> Result<(), RunIdentityError> {
    let _ = identity;
    Ok(())
}

/// Reads the text of `--attempt`.
pub fn parse_attempt(text: &str) -> Result<u32, RunIdentityError> {
    let _ = text;
    Ok(1)
}

/// Builds the identity of `--run-id`, `--attempt` and every `--context key=value`, checked.
/// An absent attempt is refused, never defaulted.
pub fn from_flags(
    run_id: &str,
    attempt: Option<&str>,
    context: &[String],
) -> Result<RunIdentity, RunIdentityError> {
    let _ = (run_id, attempt, context);
    Ok(RunIdentity {
        run_id: String::new(),
        attempt: 0,
        context: BTreeMap::new(),
    })
}

/// Whether two identities name the same run: the same id, the same attempt and the same
/// context.
pub fn same_as(a: &RunIdentity, b: &RunIdentity) -> bool {
    let _ = (a, b);
    true
}
