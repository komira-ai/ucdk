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

/// A marker line: its 1-based number, the byte offset where it starts and the one after it.
type Marker = (usize, usize, usize);

/// The shared core of `agents`: the text strictly between the line that is exactly [`BEGIN`]
/// and the line that is exactly [`END`] (lines split at `\n` only, so a carriage return or a
/// space makes a line no marker). Each marker must be there once, the begin before the end.
pub fn core(agents: &str) -> Result<Core<'_>, Vec<String>> {
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
        ([begin], [end]) if end.0 < begin.0 => Err(vec![format!(
            "{AGENTS}:{}: the end of the shared core comes before its begin (line {})",
            end.0, begin.0
        )]),
        ([begin], [end]) => Ok(Core {
            text: &agents[begin.2..end.1],
            lines: begin.0 + 1..end.0,
        }),
        _ => Err([(BEGIN, &begins), (END, &ends)]
            .into_iter()
            .filter(|(_, found)| found.len() != 1)
            .map(|(marker, found)| {
                format!(
                    "{AGENTS}: the line `{marker}` is there {} times, not once",
                    found.len()
                )
            })
            .collect()),
    }
}

/// Nothing when the core is byte for byte `copy`; otherwise the first line where they differ,
/// named by its number in both files and quoted with its newline, so a difference in the
/// final newline or a carriage return shows.
pub fn same_core(core: &Core<'_>, copy: &str) -> Result<(), String> {
    if core.text == copy {
        return Ok(());
    }
    let got: Vec<&str> = core.text.split_inclusive('\n').collect();
    let want: Vec<&str> = copy.split_inclusive('\n').collect();
    let i = got.iter().zip(&want).take_while(|(a, b)| a == b).count();
    let show =
        |line: Option<&&str>| line.map_or("the end of the text".to_owned(), |l| format!("{l:?}"));
    Err(format!(
        "{AGENTS}:{}: the shared core differs from {COPY}:{}: {AGENTS} has {}, the copy has {}",
        core.lines.start + i,
        i + 1,
        show(got.get(i)),
        show(want.get(i)),
    ))
}

/// The line `sha256sum shared_core.md` writes: the digest in lower-case hex, two spaces, the
/// file name and a newline.
pub fn hash_line(copy: &[u8]) -> String {
    format!("{}  shared_core.md\n", sha256_hex(copy))
}

/// Nothing when `hash_file` is exactly [`hash_line`] of `copy`.
pub fn hash_matches(copy: &[u8], hash_file: &str) -> Result<(), String> {
    let want = hash_line(copy);
    if hash_file == want {
        Ok(())
    } else {
        Err(format!(
            "{HASH} is {hash_file:?}, not {want:?} (run `sha256sum shared_core.md` in docs/agents)"
        ))
    }
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
