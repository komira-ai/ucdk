// SPDX-License-Identifier: Apache-2.0
//! The file format of the file ledger: a line-oriented text document.
//!
//! ```text
//! ucdk-ledger 1.0
//! version 7
//! record app/run
//! kind service
//! name app-run
//! origin created
//! state confirmed
//! physical id-1
//! dep db/run
//! output URL some-value
//! end 1 0123456789abcdef
//! ```
//!
//! Every value is escaped so that it is one token: bytes other than letters, digits and
//! `._-/:` become `%XX`, and the empty text is a lone `%`. The header carries `major.minor`.
//! A reader refuses a newer major (the layout changed) and ignores tags it does not know
//! (a minor adds tags, never changes one). The `end` line carries the number of records and
//! an FNV-1a checksum of every byte before it, so a document cut short or changed by a byte is
//! refused rather than read. The checksum guards against accidents, not against an attacker.
//! The owner of a record is not written: it is the owner path of the record's node path.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use ucdk_provider::{Kind, NodePath, PhysicalId};

use crate::record::{Origin, OwnerPath, Record, State};

/// The major version this build writes and reads. A change of layout raises it.
pub(crate) const MAJOR: u32 = 1;
/// The minor version this build writes. A minor only adds tags.
pub(crate) const MINOR: u32 = 0;

const MAGIC: &str = "ucdk-ledger";

/// A document read from a file.
#[derive(Debug)]
pub(crate) struct Parsed {
    pub(crate) minor: u32,
    pub(crate) version: u64,
    pub(crate) records: BTreeMap<NodePath, Record>,
}

/// Why a document was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormatError {
    /// The document is not a whole, valid ledger. `line` counts from 1.
    Corrupt { line: usize, why: String },
    /// The document was written by a newer major version of the format.
    NewerMajor { found: u32 },
}

fn corrupt<T>(line: usize, why: impl Into<String>) -> Result<T, FormatError> {
    Err(FormatError::Corrupt {
        line,
        why: why.into(),
    })
}

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn escape(text: &str) -> String {
    if text.is_empty() {
        return "%".to_owned();
    }
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"._-/:".contains(&byte) {
            out.push(char::from(byte));
        } else {
            write!(out, "%{byte:02X}").expect("writing to a string cannot fail");
        }
    }
    out
}

fn unescape(token: &str) -> Option<String> {
    match token {
        "" => return None,
        "%" => return Some(String::new()),
        _ => {}
    }
    let bytes = token.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = token.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn origin_text(origin: Origin) -> &'static str {
    match origin {
        Origin::Created => "created",
        Origin::Adopted => "adopted",
    }
}

fn state_text(state: State) -> &'static str {
    match state {
        State::Intent => "intent",
        State::Confirmed => "confirmed",
        State::Reaping => "reaping",
    }
}

/// The document for `records` at `version`, as the bytes of a file.
pub(crate) fn encode(version: u64, records: &BTreeMap<NodePath, Record>) -> String {
    let mut out = format!("{MAGIC} {MAJOR}.{MINOR}\nversion {version}\n");
    for record in records.values() {
        let mut line = |text: String| {
            out.push_str(&text);
            out.push('\n');
        };
        line(format!("record {}", escape(record.path.as_str())));
        line(format!("kind {}", escape(record.kind.as_str())));
        line(format!("name {}", escape(&record.name)));
        line(format!("origin {}", origin_text(record.origin)));
        line(format!("state {}", state_text(record.state)));
        if let Some(physical) = &record.physical {
            line(format!("physical {}", escape(physical.as_str())));
        }
        for dep in &record.deps {
            line(format!("dep {}", escape(dep.as_str())));
        }
        for (name, value) in &record.outputs {
            line(format!("output {} {}", escape(name), escape(value)));
        }
    }
    let sum = checksum(out.as_bytes());
    // Writing to a string cannot fail.
    let _ = writeln!(out, "end {} {sum:016x}", records.len());
    out
}

/// One line of a document, split into its tag and the rest.
struct Line<'a> {
    number: usize,
    tag: &'a str,
    rest: &'a str,
}

impl Line<'_> {
    /// The rest of the line as exactly `N` escaped tokens, unescaped.
    fn tokens<const N: usize>(&self) -> Result<[String; N], FormatError> {
        let mut texts = Vec::new();
        for part in self.rest.split(' ') {
            match unescape(part) {
                Some(text) => texts.push(text),
                None => return corrupt(self.number, "a value is not validly escaped"),
            }
        }
        <[String; N]>::try_from(texts)
            .or_else(|_| corrupt(self.number, format!("`{}` needs {N} value(s)", self.tag)))
    }

    fn one(&self) -> Result<String, FormatError> {
        let [text] = self.tokens::<1>()?;
        Ok(text)
    }

    fn path(&self) -> Result<NodePath, FormatError> {
        NodePath::new(&self.one()?).or_else(|e| corrupt(self.number, format!("a node path: {e}")))
    }
}

/// The fields of the record under construction, as the lines name them.
struct Pending<'a> {
    start: usize,
    path: NodePath,
    lines: Vec<Line<'a>>,
}

impl Pending<'_> {
    fn single(&self, tag: &str) -> Result<String, FormatError> {
        let mut found = self.lines.iter().filter(|l| l.tag == tag);
        match (found.next(), found.next()) {
            (Some(line), None) => line.one(),
            (None, _) => corrupt(self.start, format!("the record has no `{tag}`")),
            (Some(_), Some(again)) => corrupt(again.number, format!("`{tag}` is given twice")),
        }
    }

    fn build(self) -> Result<Record, FormatError> {
        let origin = match self.single("origin")?.as_str() {
            "created" => Origin::Created,
            "adopted" => Origin::Adopted,
            other => return corrupt(self.start, format!("`{other}` is not an origin")),
        };
        let state = match self.single("state")?.as_str() {
            "intent" => State::Intent,
            "confirmed" => State::Confirmed,
            "reaping" => State::Reaping,
            other => return corrupt(self.start, format!("`{other}` is not a state")),
        };
        let physical = self
            .lines
            .iter()
            .find(|l| l.tag == "physical")
            .map(|l| l.one().map(PhysicalId::new))
            .transpose()?;
        let mut deps = BTreeSet::new();
        let mut outputs = BTreeMap::new();
        for line in &self.lines {
            match line.tag {
                "dep" => {
                    deps.insert(line.path()?);
                }
                "output" => {
                    let [name, value] = line.tokens::<2>()?;
                    outputs.insert(name, value);
                }
                _ => {}
            }
        }
        Ok(Record {
            owner: OwnerPath::of(&self.path),
            kind: Kind::new(self.single("kind")?),
            name: self.single("name")?,
            origin,
            state,
            physical,
            deps,
            outputs,
            path: self.path,
        })
    }
}

fn header(first: &str) -> Result<(u32, u32), FormatError> {
    let version = first
        .strip_prefix(MAGIC)
        .and_then(|rest| rest.strip_prefix(' '))
        .and_then(|rest| rest.split_once('.'))
        .and_then(|(major, minor)| Some((major.parse().ok()?, minor.parse().ok()?)));
    match version {
        Some((major, minor)) if major >= MAJOR => Ok((major, minor)),
        _ => corrupt(1, "this is not a ucdk ledger"),
    }
}

/// Reads a document. The whole document is checked (header, checksum, count) before any of it
/// is believed.
pub(crate) fn decode(bytes: &[u8]) -> Result<Parsed, FormatError> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return corrupt(1, "the document is not text");
    };
    let (major, minor) = header(text.lines().next().unwrap_or(""))?;
    if major > MAJOR {
        return Err(FormatError::NewerMajor { found: major });
    }
    let all: Vec<&str> = text.split('\n').collect();
    // A whole document ends in a newline, so the last element is empty and the one before it
    // is the `end` line.
    let [body @ .., end, ""] = all.as_slice() else {
        return corrupt(all.len(), "the document is cut short (no end line)");
    };
    let end_number = body.len() + 1;
    let Some(("end", counts)) = end.split_once(' ') else {
        return corrupt(end_number, "the document is cut short (no end line)");
    };
    let before_end = text.len() - end.len() - 1;
    let wanted = counts.split_once(' ').and_then(|(count, sum)| {
        Some((
            count.parse::<usize>().ok()?,
            u64::from_str_radix(sum, 16).ok()?,
        ))
    });
    let Some((count, sum)) = wanted else {
        return corrupt(end_number, "the end line is malformed");
    };
    if sum != checksum(&bytes[..before_end]) {
        return corrupt(
            end_number,
            "the checksum does not match (the document changed)",
        );
    }

    let mut version = None;
    let mut records = BTreeMap::new();
    let mut pending: Option<Pending<'_>> = None;
    for (index, raw) in body.iter().enumerate().skip(1) {
        let number = index + 1;
        let (tag, rest) = raw.split_once(' ').unwrap_or((raw, ""));
        let line = Line { number, tag, rest };
        if tag == "record" {
            if let Some(done) = pending.take() {
                insert(&mut records, done)?;
            }
            pending = Some(Pending {
                start: number,
                path: line.path()?,
                lines: Vec::new(),
            });
        } else if let Some(open) = &mut pending {
            open.lines.push(line);
        } else if tag == "version" {
            let parsed = line.one()?.parse::<u64>();
            version = Some(parsed.or_else(|_| corrupt(number, "the version is not a number"))?);
        }
    }
    if let Some(done) = pending.take() {
        insert(&mut records, done)?;
    }
    let Some(version) = version else {
        return corrupt(end_number, "the document has no version");
    };
    if records.len() != count {
        return corrupt(end_number, "the record count does not match");
    }
    Ok(Parsed {
        minor,
        version,
        records,
    })
}

fn insert(
    records: &mut BTreeMap<NodePath, Record>,
    pending: Pending<'_>,
) -> Result<(), FormatError> {
    let start = pending.start;
    let record = pending.build()?;
    match records.insert(record.path.clone(), record) {
        None => Ok(()),
        Some(_) => corrupt(start, "a record is given twice"),
    }
}
