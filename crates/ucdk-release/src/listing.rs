// SPDX-License-Identifier: Apache-2.0
//! Reading the output listing of a build system: the one output of a target, from what the
//! build system itself printed and never from a guessed path.
//!
//! Buck2 prints one line per output, the label, a space and the absolute path (the path may
//! hold spaces, so a line is split at the first space). Bazel prints the output files of the
//! target as paths relative to the execution root (`cquery --output=files`) and the root itself
//! (`info execution_root`), and the path of an output is the root joined with the relative
//! path. A target has exactly one output: none, or more than one, is a fault, and so is a path
//! that is not absolute or climbs with `..`, which the files seam would refuse anyway.

use std::fmt;

use ucdk_release_proto::v1::Reason;

/// The one output of a target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The label as the listing names it (Buck2) or as asked (Bazel).
    pub label: String,
    /// Absolute, without a `..` part.
    pub path: String,
}

/// Why a listing does not give one output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListingError {
    /// The listing names no output.
    None { label: String },
    /// The listing names more than one output.
    Several { label: String, found: Vec<String> },
    /// A line or a root that cannot be read as an output.
    Malformed { label: String, why: String },
}

impl ListingError {
    /// The reason a verdict carries.
    pub fn reason(&self) -> Reason {
        Reason::BuildOutput
    }
}

impl fmt::Display for ListingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListingError::None { label } => write!(f, "{label} lists no output"),
            ListingError::Several { label, found } => write!(
                f,
                "{label} lists {} outputs, and a target has exactly one: {}",
                found.len(),
                found.join(", ")
            ),
            ListingError::Malformed { label, why } => {
                write!(f, "the listing of {label} cannot be read: {why}")
            }
        }
    }
}

fn malformed(label: &str, why: impl Into<String>) -> ListingError {
    ListingError::Malformed {
        label: label.to_owned(),
        why: why.into(),
    }
}

/// Whether `path` is absolute and climbs nowhere.
fn absolute(path: &str) -> bool {
    path.starts_with('/') && !climbs(path)
}

fn climbs(path: &str) -> bool {
    path.split('/').any(|part| part == "..")
}

/// The non-blank lines of one listing.
fn lines<'a>(label: &str, listing: &'a [u8]) -> Result<Vec<&'a str>, ListingError> {
    let text = std::str::from_utf8(listing).map_err(|_| malformed(label, "it is not UTF-8"))?;
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect())
}

/// The one output of the Buck2 listing of `label` (`build --show-full-output`).
pub fn buck2(label: &str, stdout: &[u8]) -> Result<Output, ListingError> {
    let mut found = Vec::new();
    for line in lines(label, stdout)? {
        let (listed, path) = line
            .split_once(' ')
            .ok_or_else(|| malformed(label, format!("the line {line:?} has no space")))?;
        if !absolute(path) {
            return Err(malformed(
                label,
                format!("the path {path:?} is not absolute, or climbs"),
            ));
        }
        found.push(Output {
            label: listed.to_owned(),
            path: path.to_owned(),
        });
    }
    one(label, found)
}

/// The one output of the Bazel listing of `label`: `query` is what `cquery --output=files`
/// printed and `info` what `info execution_root` printed.
pub fn bazel(label: &str, query: &[u8], info: &[u8]) -> Result<Output, ListingError> {
    let roots = lines(label, info)?;
    let root = match roots.as_slice() {
        [root] if absolute(root.trim_end_matches('/')) => root.trim_end_matches('/'),
        _ => {
            return Err(malformed(
                label,
                "the execution root is not one absolute path",
            ));
        }
    };
    let mut found = Vec::new();
    for rel in lines(label, query)? {
        if rel.starts_with('/') || climbs(rel) {
            return Err(malformed(
                label,
                format!("the output {rel:?} is not relative, or climbs"),
            ));
        }
        found.push(Output {
            label: label.to_owned(),
            path: format!("{root}/{rel}"),
        });
    }
    one(label, found)
}

fn one(label: &str, mut found: Vec<Output>) -> Result<Output, ListingError> {
    match found.len() {
        0 => Err(ListingError::None {
            label: label.to_owned(),
        }),
        1 => Ok(found.remove(0)),
        _ => Err(ListingError::Several {
            label: label.to_owned(),
            found: found.into_iter().map(|o| o.path).collect(),
        }),
    }
}
