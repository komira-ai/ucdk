// SPDX-License-Identifier: Apache-2.0
//! The repository checks behind `cargo xtask`. Each check is a function that returns its
//! findings, so a test calls it without spawning a process.

use std::fmt;
use std::path::{Path, PathBuf};

pub mod check;
pub mod cli;
pub mod cmd;
pub mod coverage;
pub mod dco;
pub mod direction;
pub mod event;
pub mod fetch;
pub mod lint;
pub mod purity;
pub mod runlines;
pub mod spdx;
pub mod tree;

/// One thing a check refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// A stable rule name; tests match on it.
    pub rule: &'static str,
    /// What the rule found fault with: a file, a commit, a crate, a command.
    pub subject: String,
    pub detail: String,
}

impl Finding {
    pub fn new(rule: &'static str, subject: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            rule,
            subject: subject.into(),
            detail: detail.into(),
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}: {}", self.rule, self.subject, self.detail)
    }
}

/// The workspace root, fixed at compile time (xtask lives directly under it).
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

/// Turns an I/O error into the `String` error the verbs report.
#[allow(clippy::needless_pass_by_value)] // used as a `map_err` function
pub(crate) fn io_err(e: std::io::Error) -> String {
    e.to_string()
}
