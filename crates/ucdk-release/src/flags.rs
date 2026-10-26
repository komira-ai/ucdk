// SPDX-License-Identifier: Apache-2.0
//! The text rules of the flags and environment a machine authors for its build system: which
//! flags ucdk owns and an author may not write, which braces a flag or an environment value may
//! hold, and which environment keys may be named. The machine rules `machine.reserved_flag`,
//! `machine.placeholder` and `machine.env_key` are these functions applied to a whole machine.
//!
//! Every function here is pure and reads one element at a time, because a flag is one argv
//! element.

use std::fmt;

use ucdk_release_proto::v1::BuildSystemKind;

/// The flags Buck2 owns on a verb of ucdk's, written without the leading `--`.
const BUCK2_OWNED: [&str; 6] = [
    "show-output",
    "show-full-output",
    "show-simple-output",
    "show-json-output",
    "show-full-json-output",
    "event-log",
];

/// The flags Bazel owns on a verb of ucdk's, written without the leading `--`.
const BAZEL_OWNED: [&str; 5] = [
    "output",
    "build_event_json_file",
    "build_event_text_file",
    "build_event_binary_file",
    "symlink_prefix",
];

/// The flags ucdk owns for `kind`, without the leading `--`; none for a kind that is not a
/// build system.
pub fn owned_flags(kind: BuildSystemKind) -> &'static [&'static str] {
    match kind {
        BuildSystemKind::Buck2 => &BUCK2_OWNED,
        BuildSystemKind::Bazel => &BAZEL_OWNED,
        BuildSystemKind::Unspecified => &[],
    }
}

/// Why one flag element is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reserved {
    /// A bare `--`, which would end option parsing before ucdk's own flags.
    EndOfOptions,
    /// A Buck2 element starting with `@`, which names an argument file ucdk cannot read.
    ArgFile,
    /// The element names, or may be read as, a flag ucdk owns.
    Owned(&'static str),
}

impl fmt::Display for Reserved {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reserved::EndOfOptions => {
                f.write_str("`--` ends option parsing, so ucdk's own flags could not follow it")
            }
            Reserved::ArgFile => f.write_str("an argument file may hold a flag ucdk owns"),
            Reserved::Owned(flag) => write!(f, "`--{flag}` is a flag ucdk owns"),
        }
    }
}

/// Reads one element of a flag list for `kind` and answers why it is refused, or `None` when
/// it is accepted.
///
/// The rule, element by element:
/// 1. `--` is refused for both kinds.
/// 2. For Buck2, an element starting with `@` is refused.
/// 3. An element that does not start with `--` is a value, a short option or a word, and is
///    accepted.
/// 4. Otherwise the name is the text after `--` up to the first `=`; an empty name is accepted.
///    The name and the owned flags are compared with `-` read as `_`.
/// 5. The candidates are the name and, when it starts with `no`, the name without that `no` and
///    one following `_`; an empty candidate is dropped.
/// 6. The element is refused when a candidate equals an owned flag of the kind. For Buck2 it is
///    refused as well when a candidate is a strict prefix of an owned flag, because its parser
///    may infer a long flag from a prefix; for Bazel a strict prefix is accepted. A longer name
///    that only starts like an owned flag is never matched.
///
/// A kind that is not a build system owns nothing and accepts every element but `--`.
pub fn reserved(kind: BuildSystemKind, element: &str) -> Option<Reserved> {
    if element == "--" {
        return Some(Reserved::EndOfOptions);
    }
    if kind == BuildSystemKind::Buck2 && element.starts_with('@') {
        return Some(Reserved::ArgFile);
    }
    let rest = element.strip_prefix("--")?;
    let name = rest.split_once('=').map_or(rest, |(name, _)| name);
    if name.is_empty() {
        return None;
    }
    let name = name.replace('-', "_");
    let negated = name
        .strip_prefix("no")
        .map(|tail| tail.strip_prefix('_').unwrap_or(tail))
        .filter(|tail| !tail.is_empty());
    let infers_prefix = kind == BuildSystemKind::Buck2;
    owned_flags(kind).iter().find_map(|owned| {
        let spelled = owned.replace('-', "_");
        [Some(name.as_str()), negated]
            .into_iter()
            .flatten()
            .any(|candidate| {
                candidate == spelled || (infers_prefix && spelled.starts_with(candidate))
            })
            .then_some(Reserved::Owned(owned))
    })
}

/// The values a placeholder stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Placeholder {
    ReleaseVersion,
    BuildNumber,
    SourceCommit,
    Revision,
    TimestampMs,
    Platform,
}

impl Placeholder {
    /// The closed list, in the order the design gives it.
    pub const ALL: [Placeholder; 6] = [
        Placeholder::ReleaseVersion,
        Placeholder::BuildNumber,
        Placeholder::SourceCommit,
        Placeholder::Revision,
        Placeholder::TimestampMs,
        Placeholder::Platform,
    ];

    /// The name between the braces.
    pub fn name(self) -> &'static str {
        match self {
            Placeholder::ReleaseVersion => "release_version",
            Placeholder::BuildNumber => "build_number",
            Placeholder::SourceCommit => "source_commit",
            Placeholder::Revision => "revision",
            Placeholder::TimestampMs => "timestamp_ms",
            Placeholder::Platform => "platform",
        }
    }

    fn named(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == text)
    }
}

/// A brace the scanner refuses, with the byte offset of the brace that opens the problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BraceError {
    /// `{name}` where `name` is not one of the six placeholders (an empty name included).
    Unknown { at: usize, name: String },
    /// A `{` that no `}` closes before the text ends or another `{` opens.
    Unclosed { at: usize },
    /// A `}` that closes nothing and is not doubled.
    Stray { at: usize },
}

impl fmt::Display for BraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BraceError::Unknown { at, name } => {
                write!(
                    f,
                    "`{{{name}}}` at byte {at} is not a placeholder (the names are {})",
                    Placeholder::ALL.map(Placeholder::name).join(", ")
                )
            }
            BraceError::Unclosed { at } => {
                write!(
                    f,
                    "the `{{` at byte {at} is never closed (write `{{{{` for a brace)"
                )
            }
            BraceError::Stray { at } => {
                write!(
                    f,
                    "the `}}` at byte {at} closes nothing (write `}}}}` for a brace)"
                )
            }
        }
    }
}

/// The placeholders of one flag or environment value, in the order they occur, or the first
/// brace that is neither a placeholder nor a doubled brace. `{{` and `}}` are literal braces.
pub fn placeholders(text: &str) -> Result<Vec<Placeholder>, BraceError> {
    let mut found = Vec::new();
    walk(text, |piece| {
        if let Piece::Holder(p) = piece {
            found.push(p);
        }
    })?;
    Ok(found)
}

/// `text` with each placeholder replaced by what `value` answers for it and each doubled brace
/// replaced by one literal brace, or the first brace that is neither (the scanner of
/// [`placeholders`] reads it, so a text the machine rules accept is a text this accepts).
pub fn substitute(text: &str, value: impl Fn(Placeholder) -> String) -> Result<String, BraceError> {
    let mut out = String::with_capacity(text.len());
    walk(text, |piece| match piece {
        Piece::Text(literal) => out.push_str(literal),
        Piece::Holder(p) => out.push_str(&value(p)),
    })?;
    Ok(out)
}

/// One piece of a scanned text.
enum Piece<'t> {
    /// Literal text; a doubled brace gives the one brace it stands for.
    Text(&'t str),
    Holder(Placeholder),
}

/// The scanner: hands `sink` the pieces of `text` in order, or fails at the first brace that is
/// neither a placeholder nor a doubled brace.
fn walk<'t>(text: &'t str, mut sink: impl FnMut(Piece<'t>)) -> Result<(), BraceError> {
    let bytes = text.as_bytes();
    let mut at = 0;
    // Where the literal text not yet handed to `sink` begins.
    let mut run = 0;
    while at < bytes.len() {
        match bytes[at] {
            brace @ (b'{' | b'}') if bytes.get(at + 1) == Some(&brace) => {
                // The literal text up to and including the first brace of the pair.
                sink(Piece::Text(&text[run..=at]));
                at += 2;
                run = at;
            }
            b'}' => return Err(BraceError::Stray { at }),
            b'{' => {
                let close = bytes[at + 1..]
                    .iter()
                    .position(|b| matches!(b, b'{' | b'}'))
                    .filter(|offset| bytes[at + 1 + offset] == b'}')
                    .ok_or(BraceError::Unclosed { at })?;
                let name = &text[at + 1..at + 1 + close];
                let Some(p) = Placeholder::named(name) else {
                    return Err(BraceError::Unknown {
                        at,
                        name: name.to_owned(),
                    });
                };
                sink(Piece::Text(&text[run..at]));
                sink(Piece::Holder(p));
                at += close + 2;
                run = at;
            }
            _ => at += 1,
        }
    }
    sink(Piece::Text(&text[run..]));
    Ok(())
}

/// Why an environment key is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvKeyError {
    /// Not `[A-Za-z_][A-Za-z0-9_]*`.
    Grammar,
    /// Starts with `UCDK_` in any letter case, which ucdk keeps for itself.
    Reserved,
}

impl fmt::Display for EnvKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            EnvKeyError::Grammar => "an environment key is [A-Za-z_][A-Za-z0-9_]*",
            EnvKeyError::Reserved => "an environment key may not start with UCDK_ in any case",
        })
    }
}

/// Checks one environment key. Lower case is allowed, so `http_proxy` and `no_proxy` can be
/// named.
pub fn env_key(key: &str) -> Result<(), EnvKeyError> {
    let bytes = key.as_bytes();
    let first_ok = bytes
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_');
    if !first_ok
        || !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        return Err(EnvKeyError::Grammar);
    }
    if bytes.len() >= 5 && bytes[..5].eq_ignore_ascii_case(b"UCDK_") {
        return Err(EnvKeyError::Reserved);
    }
    Ok(())
}
