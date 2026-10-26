// SPDX-License-Identifier: Apache-2.0
//! The names of things in the artifact store. A set is built into a generation, and every key
//! is a function of the stage, platform, revision and generation of the set and of the name of
//! the member, so two revisions, two platforms or two runs never meet at one key.

use std::fmt;

/// Which set a key belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetScope {
    pub stage: String,
    pub platform: String,
    pub revision: String,
    pub generation: String,
}

/// A key in the store: `<stage>/<platform>/<revision>/<generation>/m/<member>` for a member and
/// `<stage>/<platform>/<revision>/<generation>/set` for the set document. It is made only by
/// [`StoreKey::member`] and [`StoreKey::set`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoreKey(String);

/// A part of a key that is empty, is `.` or `..`, or holds a `/`, a `\` or a control character,
/// so the key would not say what it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreKeyError {
    pub part: &'static str,
}

impl fmt::Display for StoreKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the {} of a store key is empty, is a dot part, or holds a `/`, a `\\` or a control character",
            self.part
        )
    }
}

impl SetScope {
    /// The start of every key of this generation, up to and including the last `/`.
    pub fn prefix(&self) -> Result<String, StoreKeyError> {
        join(self, None).map(|head| format!("{head}/"))
    }
}

/// The scope, and the member name when there is one, joined with `/`.
fn join(scope: &SetScope, member: Option<&str>) -> Result<String, StoreKeyError> {
    let parts = [
        ("stage", Some(scope.stage.as_str())),
        ("platform", Some(scope.platform.as_str())),
        ("revision", Some(scope.revision.as_str())),
        ("generation", Some(scope.generation.as_str())),
        ("member", member),
    ];
    let given: Vec<(&'static str, &str)> = parts
        .iter()
        .filter_map(|(what, part)| part.map(|p| (*what, p)))
        .collect();
    if let Some((part, _)) = given.iter().find(|(_, p)| !is_plain(p)) {
        return Err(StoreKeyError { part });
    }
    let head: Vec<&str> = given.iter().take(4).map(|(_, p)| *p).collect();
    Ok(match given.get(4) {
        Some((_, name)) => format!("{}/m/{name}", head.join("/")),
        None => head.join("/"),
    })
}

/// A part that names one thing: not empty, not `.` or `..`, and with no `/`, `\` or control
/// character, so it cannot stand for a parent, a root or a second part.
fn is_plain(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
}

impl StoreKey {
    /// The key of the member `name` of the set at `scope`.
    pub fn member(scope: &SetScope, name: &str) -> Result<Self, StoreKeyError> {
        join(scope, Some(name)).map(Self)
    }

    /// The key of the set document of the set at `scope`.
    pub fn set(scope: &SetScope) -> Result<Self, StoreKeyError> {
        join(scope, None).map(|head| Self(format!("{head}/set")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StoreKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
