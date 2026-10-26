// SPDX-License-Identifier: Apache-2.0
//! The layering of one build-system invocation: the argument vector and the environment of one
//! call of the build system, a pure function of the build system as authored, a target, the
//! verb, the build stamp, the revision and the platform.
//!
//! The argument vector is these elements in this order and nothing else: the binary; the
//! kind's startup arguments from `config_file`; the verb word; the kind's command arguments
//! from `config_file` and `mode`; the base flags; the target's flags; the flags ucdk owns for
//! the kind and verb; the tail of the verb. Flags are appended and never merged, de-duplicated
//! or interpreted. The environment is the base environment with the target's over it key by
//! key, and nothing is inherited. Placeholders in flags and environment values are substituted
//! through [`crate::flags::substitute`]; the binary, config file, mode and labels are not.

use std::collections::BTreeMap;
use std::fmt;

use ucdk_release_proto::v1::{BuildStamp, BuildSystem, BuildSystemKind, Target};

use crate::flags::{BraceError, Placeholder, substitute};

/// What one invocation does. `Query` and `Info` are Bazel's two calls that locate an output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Build,
    Test,
    Query,
    Info,
}

impl fmt::Display for Verb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Verb::Build => "build",
            Verb::Test => "test",
            Verb::Query => "query",
            Verb::Info => "info",
        })
    }
}

/// Everything the layering reads.
#[derive(Debug, Clone, Copy)]
pub struct Invocation<'a> {
    /// The machine's build system, as authored.
    pub system: &'a BuildSystem,
    pub target: &'a Target,
    pub verb: Verb,
    /// A `Test` invocation's report file, which the verb's owned flag names; `None` for every
    /// other verb. The path is the engine's, neither authored nor hashed.
    pub report: Option<&'a str>,
    pub stamp: &'a BuildStamp,
    /// The run's revision, for `{revision}`.
    pub revision: &'a str,
    /// The stage's platform, for `{platform}`; empty when the stage names none.
    pub platform: &'a str,
}

/// An argument vector and the environment of the child, which inherits nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub argv: Vec<String>,
    /// In sorted key order.
    pub env: Vec<(String, String)>,
}

/// Why an invocation has no layering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationError {
    /// The build system's kind is not one of the two.
    Kind,
    /// The kind has no such verb (Buck2 has no `Query` or `Info`).
    Verb { kind: BuildSystemKind, verb: Verb },
    /// A `Test` without a report path, or another verb with one.
    Report { verb: Verb, given: bool },
    /// A flag or an environment value holds a brace that is neither a placeholder nor a doubled
    /// brace.
    Brace { at: String, error: BraceError },
}

impl fmt::Display for InvocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvocationError::Kind => f.write_str("the build system names no known kind"),
            InvocationError::Verb { kind, verb } => {
                write!(f, "{kind:?} has no `{verb}` invocation")
            }
            InvocationError::Report { verb, given: true } => {
                write!(f, "a `{verb}` invocation writes no report")
            }
            InvocationError::Report { verb, given: false } => {
                write!(f, "a `{verb}` invocation needs the path of its report")
            }
            InvocationError::Brace { at, error } => write!(f, "{at}: {error}"),
        }
    }
}

/// The fixed name of the report file of a `Test` invocation, which the engine puts in a
/// directory of its own. Buck2 chooses the encoding and compression of an event log from the
/// suffix of its name, so the name ends in `.json-lines` (uncompressed JSON lines); the name
/// for Bazel only says what the file holds. `None` for a kind that is not a build system.
pub fn report_file_name(kind: BuildSystemKind) -> Option<&'static str> {
    match kind {
        BuildSystemKind::Buck2 => Some("events.json-lines"),
        BuildSystemKind::Bazel => Some("events.json"),
        BuildSystemKind::Unspecified => None,
    }
}

/// The verb word, the flags ucdk owns for it and its tail.
struct Verbed {
    word: &'static str,
    owned: Vec<String>,
    tail: Vec<String>,
}

fn label_tail(kind: BuildSystemKind, label: &str) -> Vec<String> {
    match kind {
        BuildSystemKind::Bazel => vec!["--".to_owned(), label.to_owned()],
        _ => vec![label.to_owned()],
    }
}

fn verbed(kind: BuildSystemKind, inv: &Invocation<'_>) -> Result<Verbed, InvocationError> {
    let label = &inv.target.label;
    let report = match (inv.verb, inv.report) {
        (Verb::Test, Some(path)) => path,
        (verb @ Verb::Test, None) => return Err(InvocationError::Report { verb, given: false }),
        (verb, Some(_)) => return Err(InvocationError::Report { verb, given: true }),
        (_, None) => "",
    };
    let (word, owned, tail) = match (kind, inv.verb) {
        (BuildSystemKind::Buck2, Verb::Build) => (
            "build",
            vec!["--show-full-output".to_owned()],
            label_tail(kind, label),
        ),
        (BuildSystemKind::Buck2, Verb::Test) => (
            "test",
            vec![format!("--event-log={report}")],
            label_tail(kind, label),
        ),
        (BuildSystemKind::Bazel, Verb::Build) => ("build", Vec::new(), label_tail(kind, label)),
        (BuildSystemKind::Bazel, Verb::Query) => (
            "cquery",
            vec!["--output=files".to_owned()],
            label_tail(kind, label),
        ),
        (BuildSystemKind::Bazel, Verb::Info) => {
            ("info", Vec::new(), vec!["execution_root".to_owned()])
        }
        (BuildSystemKind::Bazel, Verb::Test) => (
            "test",
            vec![format!("--build_event_json_file={report}")],
            label_tail(kind, label),
        ),
        (BuildSystemKind::Buck2, verb) => return Err(InvocationError::Verb { kind, verb }),
        (BuildSystemKind::Unspecified, _) => return Err(InvocationError::Kind),
    };
    Ok(Verbed { word, owned, tail })
}

/// The argument vector and the environment of `inv`.
pub fn command(inv: &Invocation<'_>) -> Result<Command, InvocationError> {
    let kind = BuildSystemKind::try_from(inv.system.kind).unwrap_or(BuildSystemKind::Unspecified);
    let verbed = verbed(kind, inv)?;
    let value = |p: Placeholder| match p {
        Placeholder::ReleaseVersion => inv.stamp.version.clone(),
        Placeholder::BuildNumber => inv.stamp.build_number.to_string(),
        Placeholder::SourceCommit => inv.stamp.source_commit.clone(),
        Placeholder::Revision => inv.revision.to_owned(),
        Placeholder::TimestampMs => inv.stamp.timestamp_ms.to_string(),
        Placeholder::Platform => inv.platform.to_owned(),
    };
    let flags = |list: &[String], whose: &str| -> Result<Vec<String>, InvocationError> {
        list.iter()
            .enumerate()
            .map(|(i, flag)| {
                substitute(flag, value).map_err(|error| InvocationError::Brace {
                    at: format!("{whose} flag {i}"),
                    error,
                })
            })
            .collect()
    };
    let system = inv.system;
    let mut argv = vec![system.binary.clone()];
    if kind == BuildSystemKind::Bazel && !system.config_file.is_empty() {
        argv.push(format!("--bazelrc={}", system.config_file));
    }
    argv.push(verbed.word.to_owned());
    match kind {
        BuildSystemKind::Buck2 => {
            if !system.config_file.is_empty() {
                argv.push("--config-file".to_owned());
                argv.push(system.config_file.clone());
            }
            if !system.mode.is_empty() {
                argv.push(format!("@{}", system.mode));
            }
        }
        _ => {
            if !system.mode.is_empty() {
                argv.push(format!("--config={}", system.mode));
            }
        }
    }
    argv.extend(flags(&system.flags, "the base")?);
    argv.extend(flags(&inv.target.flags, "the target's")?);
    argv.extend(verbed.owned);
    argv.extend(verbed.tail);

    let mut env: BTreeMap<&str, &str> = system
        .env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    env.extend(inv.target.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let env = env
        .into_iter()
        .map(|(key, raw)| {
            substitute(raw, value)
                .map(|text| (key.to_owned(), text))
                .map_err(|error| InvocationError::Brace {
                    at: format!("the environment value of {key}"),
                    error,
                })
        })
        .collect::<Result<_, _>>()?;
    Ok(Command { argv, env })
}
