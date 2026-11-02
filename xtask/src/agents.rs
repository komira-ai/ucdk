// SPDX-License-Identifier: Apache-2.0
//! `AGENTS.md`, the rules every change is written and reviewed against, and the files that keep
//! its shared core. The core is the part that is the same text in every repository of the
//! project; it sits between two marker lines of `AGENTS.md`. [`COPY`] is the canonical copy of
//! that text and [`HASH`] the copy's `sha256sum` line, so an edit to the core in `AGENTS.md`
//! alone, or to it and the copy together, is seen. `CLAUDE.md` imports `AGENTS.md` with a line
//! [`IMPORT`].
//!
//! The functions read the texts and nothing else; `xtask/tests/agents_md.rs` holds the real
//! files to them, and the public-repository lint finds the core's lines with [`core`].

use std::fmt;
use std::ops::Range;

use crate::fetch::sha256_hex;

pub const AGENTS: &str = "AGENTS.md";
pub const CLAUDE: &str = "CLAUDE.md";
/// The canonical copy of the shared core.
pub const COPY: &str = "docs/agents/shared_core.md";
/// The copy's hash, in the form `sha256sum shared_core.md` writes in `docs/agents`.
pub const HASH: &str = "docs/agents/shared_core.sha256";
pub const BEGIN: &str = "<!-- shared-core:begin -->";
pub const END: &str = "<!-- shared-core:end -->";
/// The line of `CLAUDE.md` that imports `AGENTS.md`.
pub const IMPORT: &str = "@AGENTS.md";

/// The shared core of an `AGENTS.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Core<'a> {
    /// The text after the begin line, up to the end line; each line keeps its `\n`.
    pub text: &'a str,
    /// The 1-based numbers of the lines of `text` in the `AGENTS.md`.
    pub lines: Range<usize>,
}

/// Why the shared core of an `AGENTS.md`, its copy or the copy's hash is refused. Its
/// `Display` is the diagnostic a person reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentsError {
    /// A marker line is not there exactly once: how many begin and end lines there are.
    MarkerCount { begins: usize, ends: usize },
    /// The end marker comes before the begin marker: the 1-based line of each.
    MarkersOutOfOrder { begin: usize, end: usize },
    /// The core differs from the copy. The first differing line, by its 1-based number in
    /// `AGENTS.md` and in the copy, and that line with its newline in each (`None` past the end).
    CoreDiffers {
        agents_line: usize,
        copy_line: usize,
        agents: Option<String>,
        copy: Option<String>,
    },
    /// The hash file is not the copy's `sha256sum` line: what it holds and what it should.
    HashMismatch { found: String, want: String },
}

impl fmt::Display for AgentsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MarkerCount { begins, ends } => {
                let wrong: Vec<String> = [(BEGIN, begins), (END, ends)]
                    .into_iter()
                    .filter(|(_, n)| **n != 1)
                    .map(|(marker, n)| {
                        format!("{AGENTS}: the line `{marker}` is there {n} times, not once")
                    })
                    .collect();
                write!(f, "{}", wrong.join("\n"))
            }
            Self::MarkersOutOfOrder { begin, end } => write!(
                f,
                "{AGENTS}:{end}: the end of the shared core comes before its begin (line {begin})"
            ),
            Self::CoreDiffers {
                agents_line,
                copy_line,
                agents,
                copy,
            } => {
                let show = |line: &Option<String>| {
                    line.as_ref()
                        .map_or("the end of the text".to_owned(), |l| format!("{l:?}"))
                };
                write!(
                    f,
                    "{AGENTS}:{agents_line}: the shared core differs from {COPY}:{copy_line}: {AGENTS} has {}, the copy has {}",
                    show(agents),
                    show(copy)
                )
            }
            Self::HashMismatch { found, want } => write!(
                f,
                "{HASH} is {found:?}, not {want:?} (run `sha256sum shared_core.md` in docs/agents)"
            ),
        }
    }
}

impl std::error::Error for AgentsError {}

/// A marker line: its 1-based number, the byte offset where it starts and the one after it.
type Marker = (usize, usize, usize);

/// The shared core of `agents`: the text strictly between the line that is exactly [`BEGIN`]
/// and the line that is exactly [`END`] (lines split at `\n` only, so a carriage return or a
/// space makes a line no marker). Each marker must be there once, the begin before the end.
pub fn core(agents: &str) -> Result<Core<'_>, AgentsError> {
    let (mut begins, mut ends): (Vec<Marker>, Vec<Marker>) = (Vec::new(), Vec::new());
    let mut start = 0;
    for (i, piece) in agents.split_inclusive('\n').enumerate() {
        let line = piece.strip_suffix('\n').unwrap_or(piece);
        let at = (i + 1, start, start + piece.len());
        if line == BEGIN {
            begins.push(at);
        } else if line == END {
            ends.push(at);
        }
        start += piece.len();
    }
    match (begins.as_slice(), ends.as_slice()) {
        ([begin], [end]) if end.0 < begin.0 => Err(AgentsError::MarkersOutOfOrder {
            begin: begin.0,
            end: end.0,
        }),
        ([begin], [end]) => Ok(Core {
            text: &agents[begin.2..end.1],
            lines: begin.0 + 1..end.0,
        }),
        _ => Err(AgentsError::MarkerCount {
            begins: begins.len(),
            ends: ends.len(),
        }),
    }
}

/// Nothing when the core is byte for byte `copy`; otherwise the first line where they differ,
/// named by its number in both files and kept with its newline, so a difference in the final
/// newline or a carriage return shows.
pub fn same_core(core: &Core<'_>, copy: &str) -> Result<(), AgentsError> {
    if core.text == copy {
        return Ok(());
    }
    let got: Vec<&str> = core.text.split_inclusive('\n').collect();
    let want: Vec<&str> = copy.split_inclusive('\n').collect();
    let i = got.iter().zip(&want).take_while(|(a, b)| a == b).count();
    Err(AgentsError::CoreDiffers {
        agents_line: core.lines.start + i,
        copy_line: i + 1,
        agents: got.get(i).map(|l| (*l).to_owned()),
        copy: want.get(i).map(|l| (*l).to_owned()),
    })
}

/// The line `sha256sum shared_core.md` writes: the digest in lower-case hex, two spaces, the
/// file name and a newline.
pub fn hash_line(copy: &[u8]) -> String {
    format!("{}  shared_core.md\n", sha256_hex(copy))
}

/// Nothing when `hash_file` is exactly [`hash_line`] of `copy`.
pub fn hash_matches(copy: &[u8], hash_file: &str) -> Result<(), AgentsError> {
    let want = hash_line(copy);
    if hash_file == want {
        Ok(())
    } else {
        Err(AgentsError::HashMismatch {
            found: hash_file.to_owned(),
            want,
        })
    }
}

/// The ids of the rules `text` states: the first word of each list item, at any indent (a
/// `-`, `*` or `+` bullet, or a number followed by `.` or `)`, then a space or a tab), with any `*`, `` ` ``, `.` or `:`
/// around it taken off, when that word is capital letters and then digits
/// (`T1`, `R10`). A reference in prose, or a list item that starts with another word, states
/// no rule.
pub fn rule_ids(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| {
            let item = line.trim_start();
            // The marker: a bullet, or digits and `.` or `)`; then a space or a tab.
            let marker = match item.chars().next() {
                Some('-' | '*' | '+') => 1,
                _ => {
                    let digits =
                        item.len() - item.trim_start_matches(|c: char| c.is_ascii_digit()).len();
                    let closed =
                        digits > 0 && matches!(item[digits..].chars().next(), Some('.' | ')'));
                    if closed { digits + 1 } else { 0 }
                }
            };
            let rest = item
                .get(marker..)
                .filter(|_| marker > 0)?
                .strip_prefix([' ', '\t'])?;
            let word = rest
                .split_whitespace()
                .next()?
                .trim_matches(['*', '`', '.', ':']);
            let letters = word.len()
                - word
                    .trim_start_matches(|c: char| c.is_ascii_uppercase())
                    .len();
            let digits = &word[letters..];
            (letters > 0 && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
                .then_some(word)
        })
        .collect()
}

/// Whether `id` is a rule kept out of `AGENTS.md`: the interface and design review (`F`, `G`,
/// and `S5`) and the hygiene layer (`H`).
pub fn withheld(id: &str) -> bool {
    id == "S5" || id.len() > 1 && matches!(id.as_bytes()[0], b'F' | b'G' | b'H')
}

/// A fence line: at most three spaces, then a run of three or more backticks or tildes (a run
/// of backticks with a backtick after it is inline code, not a fence). The fence's character,
/// the run's length and whether nothing but white space follows the run, which a closing fence
/// needs.
fn fence(line: &str) -> Option<(char, usize, bool)> {
    let text = line.trim_start_matches(' ');
    if line.len() - text.len() > 3 {
        return None;
    }
    let c = text.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    // `c` is ASCII, so the run's length in bytes is its length in characters.
    let run = text.len() - text.trim_start_matches(c).len();
    let rest = &text[run..];
    (run >= 3 && !(c == '`' && rest.contains('`'))).then(|| (c, run, rest.trim().is_empty()))
}

/// Whether `claude` has a line that is exactly [`IMPORT`] (lines split at `\n` only) outside a
/// fenced code block. A fence is closed by a fence line of the same character, at least as
/// long, with nothing after it; a fence never closed holds the rest of the text.
pub fn imports_agents(claude: &str) -> bool {
    let mut open: Option<(char, usize)> = None;
    for line in claude.split('\n') {
        match (open, fence(line)) {
            (None, Some((c, run, _))) => open = Some((c, run)),
            (Some((c, run)), Some((close, length, true))) if close == c && length >= run => {
                open = None;
            }
            (None, None) if line == IMPORT => return true,
            _ => {}
        }
    }
    false
}
