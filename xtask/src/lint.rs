// SPDX-License-Identifier: Apache-2.0
//! The public-repository lint: nothing in the tree, in a commit message or in the pull request
//! title and body may carry what a public repository must not: dates before the project's
//! public start, home paths, hostnames and addresses, account ids, e-mail addresses, other
//! organisations' names, tracker card ids and internal process words. The rules are plain
//! patterns. Text is normalised first, by dropping invisible characters and turning Unicode
//! dashes into `-`, so a split or a dash variant does not hide a date.
//!
//! This file and the licence text are exempt from the tree scan (this file's patterns would
//! match themselves, the licence carries its own date); every other file is scanned, tests
//! included, and the tests build their samples at run time. Identities are allowed in commit
//! metadata only (author, committer and the matching trailers); `trailer_name` lets exactly
//! those trailer lines through, and their name part is scanned like any other text.

use crate::Finding;
use crate::dco::CommitInfo;
use crate::event::Event;

/// Files the tree scan skips: this file (its patterns would match themselves) and the licence
/// text (which carries its own date). The tests build every sample at run time, so they are
/// scanned like any other file.
const EXEMPT: [&str; 2] = ["xtask/src/lint.rs", "LICENSE"];

/// A date is public-safe from this month on.
const START: (u32, u32) = (2026, 9);

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Home-directory prefixes, matched in any letter case.
const HOME_PATHS: [&str; 3] = ["/home/", "/users/", "c:\\users"];

/// Hosts a public file may link to; a subdomain of one is allowed too.
const ALLOWED_HOSTS: [&str; 12] = [
    "github.com",
    "nodejs.org",
    "crates.io",
    "rust-lang.org",
    "docs.rs",
    "developercertificate.org",
    "apache.org",
    "claude.com",
    "localhost",
    "example.com",
    "example.org",
    "example.net",
];
const EXAMPLE_HOSTS: [&str; 3] = ["example.com", "example.org", "example.net"];
const INTERNAL_SUFFIXES: [&str; 4] = [".internal", ".lan", ".corp", ".ts.net"];

/// Process words, matched as whole words (a plural too); built from pieces so this file does
/// not contain them whole.
const WORDS: [&str; 3] = [
    concat!("mu", "tant"),
    concat!("plan", "ted"),
    concat!("buc", "ket"),
];
/// A process word that is the start of a numbered label (this, a dash, a number).
const WORD_PREFIX: &str = concat!("m", "ut");
/// A process phrase of two words; any run of white space (or a line break) between them.
const PHRASE: [&str; 2] = ["went", concat!("r", "ed")];

type Detector = fn(&str) -> Option<String>;

const RULES: [(&str, Detector); 8] = [
    ("lint-date", old_date),
    ("lint-home-path", home_path),
    ("lint-host", host),
    ("lint-email", email),
    ("lint-account-id", account_id),
    ("lint-word", word),
    ("lint-org", other_org),
    ("lint-card-id", card_id),
];

/// The rules that also read two neighbouring lines as one.
const WRAPPED: [&str; 2] = ["lint-date", "lint-word"];

fn tokens(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
}

/// Characters that print as nothing (zero-width and joiner characters, direction marks,
/// variation selectors, combining marks); they are dropped so they cannot split a word.
fn invisible(c: char) -> bool {
    matches!(c as u32,
        0x300..=0x36f | 0xad | 0x61c | 0x115f | 0x1160 | 0x17b4 | 0x17b5
        | 0x180b..=0x180e | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f
        | 0xfe00..=0xfe0f | 0xfeff | 0x3164 | 0xffa0)
}

/// The line as a reader sees it: invisible characters dropped and the Unicode dashes and the
/// minus sign turned into `-`. Everything else is left as written.
fn fold(line: &str) -> String {
    line.chars()
        .filter(|c| !invisible(*c))
        .map(|c| match c {
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            _ => c,
        })
        .collect()
}

fn before_start(year: u32, month: u32) -> bool {
    (year, month) < START
}

fn old_date(line: &str) -> Option<String> {
    numeric_date(&glue_separators(line))
        .or_else(|| compact_date(line))
        .or_else(|| month_date(line))
}

const SEPS: [char; 3] = ['-', '/', '.'];

/// The line with the spaces around a date separator removed (`2026 - 08 - 31`), word by word.
fn glue_separators(line: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for word in line.split_whitespace() {
        match out.last_mut() {
            Some(last) if last.ends_with(SEPS) || word.starts_with(SEPS) => last.push_str(word),
            _ => out.push(word.to_owned()),
        }
    }
    out.join(" ")
}

fn year(word: &str) -> Option<u32> {
    Some(word)
        .filter(|w| w.len() == 4)
        .and_then(|w| w.parse().ok())
        .filter(|y| (1900..=2099).contains(y))
}

/// Whether three numbers, as written, are a date before the start: `YYYY-MM-DD`, or day and
/// month in either order with a four-digit year (a two-digit year only if `short_year`).
fn dated(a: &str, b: &str, c: &str, short_year: bool) -> bool {
    let num = |s: &str| s.parse::<u32>().unwrap_or(0);
    let (x, y, z) = (num(a), num(b), num(c));
    let valid = |day: u32, month: u32| (1..=31).contains(&day) && (1..=12).contains(&month);
    if a.len() == 4 {
        return year(a).is_some() && valid(z, y) && before_start(x, y);
    }
    if a.len() > 2 || b.len() > 2 {
        return false;
    }
    let full = match c.len() {
        4 => year(c),
        2 if short_year => Some(2000 + z),
        _ => None,
    };
    full.is_some_and(|yr| {
        [(x, y), (y, x)]
            .iter()
            .any(|&(day, month)| valid(day, month) && before_start(yr, month))
    })
}

/// Two numbers joined by `-` or `/`, one a four-digit year and the other a month, in either
/// order (`2026-08`, `08/2026`). A dot is left alone: `1.2024` is a version.
fn year_month(core: &str, parts: &[&str]) -> bool {
    let month = |s: &str| {
        s.parse::<u32>()
            .ok()
            .filter(|m| s.len() <= 2 && (1..=12).contains(m))
    };
    parts.len() == 2
        && !core.contains('.')
        && [(parts[0], parts[1]), (parts[1], parts[0])]
            .iter()
            .any(|(y, m)| {
                year(y)
                    .zip(month(m))
                    .is_some_and(|(y, m)| before_start(y, m))
            })
}

/// `YY-MM-DD` with a two-digit year and dashes only (`26-08-31`).
fn short_year_first(core: &str, parts: &[&str]) -> bool {
    parts.len() == 3
        && parts[0].len() == 2
        && !core.contains(['/', '.'])
        && dated(&format!("20{}", parts[0]), parts[1], parts[2], false)
}

/// Numbers joined by `-`, `/` or `.` that make a date (see `dated`), or a year and a month.
fn numeric_date(line: &str) -> Option<String> {
    let chars = |c: char| !(c.is_ascii_digit() || SEPS.contains(&c));
    line.split(chars).find_map(|token| {
        let core = token.trim_matches(SEPS);
        let parts: Vec<&str> = core.split(SEPS).filter(|p| !p.is_empty()).collect();
        let short_year = parts.len() == 3;
        (parts
            .windows(3)
            .any(|w| dated(w[0], w[1], w[2], short_year))
            || year_month(core, &parts)
            || short_year_first(core, &parts))
        .then(|| token.to_owned())
    })
}

/// `YYYYMMDD`: a word of eight digits.
fn compact_date(line: &str) -> Option<String> {
    tokens(line)
        .find(|t| {
            t.len() == 8
                && t.bytes().all(|b| b.is_ascii_digit())
                && dated(&t[..4], &t[4..6], &t[6..], false)
        })
        .map(str::to_owned)
}

/// A month name (English full, three letters or `sept`) with a four-digit
/// year within three words of it.
fn month_date(line: &str) -> Option<String> {
    let words: Vec<&str> = tokens(line).collect();
    for (i, word) in words.iter().enumerate() {
        let lower = word.to_ascii_lowercase();
        let english = MONTHS
            .iter()
            .position(|m| lower == *m || lower == m[..3] || (lower == "sept" && *m == "september"))
            .map(|p| p as u32 + 1);
        let Some(month) = english else {
            continue;
        };
        let near = &words[i.saturating_sub(3)..(i + 4).min(words.len())];
        if near
            .iter()
            .filter_map(|w| year(w))
            .any(|y| before_start(y, month))
        {
            return Some(near.join(" "));
        }
    }
    None
}

fn home_path(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    HOME_PATHS
        .iter()
        .find(|p| lower.contains(**p))
        .map(|p| (*p).to_owned())
}

fn host_allowed(host: &str, list: &[&str]) -> bool {
    let host = host.to_ascii_lowercase();
    list.iter()
        .any(|a| host == *a || host.ends_with(&format!(".{a}")))
}

/// A URL host outside the allow-list, an IPv4 address, or a name with an internal suffix.
fn host(line: &str) -> Option<String> {
    url_host(line)
        .or_else(|| ipv4(line))
        .or_else(|| suffix(line))
}

fn url_host(line: &str) -> Option<String> {
    let mut rest = line;
    while let Some(i) = rest.find("://") {
        rest = &rest[i + 3..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || ".-@_".contains(c)))
            .unwrap_or(rest.len());
        let host = rest[..end].rsplit('@').next().unwrap_or("");
        if !host.is_empty() && !host_allowed(host, &ALLOWED_HOSTS) {
            return Some(host.to_owned());
        }
    }
    None
}

fn ipv4(line: &str) -> Option<String> {
    line.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find(|t| {
            let parts: Vec<&str> = t.split('.').collect();
            parts.len() == 4 && parts.iter().all(|p| p.parse::<u8>().is_ok())
        })
        .map(str::to_owned)
}

/// The names the generated SDKs call the object whose field they read: `self` in Python, `this`
/// and the constructor's `init` in TypeScript.
const RECEIVERS: [&str; 3] = ["self.", "this.", "init."];

/// A name that ends in an internal suffix. `self.<name>` is an attribute of an object (the
/// generated SDKs have a field called `internal`), not a host.
fn suffix(line: &str) -> Option<String> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || ".-".contains(c)))
        .filter(|t| !RECEIVERS.iter().any(|r| t.starts_with(r)))
        .find(|t| INTERNAL_SUFFIXES.iter().any(|s| t.ends_with(s)))
        .map(str::to_owned)
}

/// A `local@domain.tld` address other than one at an example domain.
fn email(line: &str) -> Option<String> {
    for (i, _) in line.match_indices('@') {
        let local = line[..i]
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || "._+-".contains(*c))
            .count();
        let domain: String = line[i + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || ".-".contains(*c))
            .collect();
        if local > 0 && domain.contains('.') && !host_allowed(&domain, &EXAMPLE_HOSTS) {
            return Some(format!("an address at {domain}"));
        }
    }
    None
}

/// A twelve-digit number, or an ARN.
fn account_id(line: &str) -> Option<String> {
    let number = tokens(line).find(|t| t.len() == 12 && t.bytes().all(|b| b.is_ascii_digit()));
    number
        .map(|_| "a twelve-digit number".to_owned())
        .or_else(|| {
            line.to_ascii_lowercase()
                .contains("arn:aws:")
                .then(|| "an ARN".to_owned())
        })
}

/// A process word as a whole word (or its plural), the two-word phrase with anything but
/// letters and digits between the words, or the prefix followed by a dash.
fn word(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let words: Vec<&str> = tokens(&lower).collect();
    let whole = WORDS
        .iter()
        .find(|w| words.iter().any(|t| t == *w || *t == format!("{w}s")));
    let phrase = words.windows(2).any(|pair| pair == PHRASE);
    let prefix = lower.match_indices(WORD_PREFIX).any(|(i, _)| {
        let before = lower[..i].chars().next_back();
        let after = lower[i + WORD_PREFIX.len()..].trim_start();
        !before.is_some_and(|c| c.is_ascii_alphanumeric()) && after.starts_with('-')
    });
    match (whole, phrase, prefix) {
        (Some(w), _, _) => Some((*w).to_owned()),
        (None, true, _) => Some(PHRASE.join(" ")),
        (None, false, true) => Some(WORD_PREFIX.to_owned()),
        (None, false, false) => None,
    }
}

/// The only organisation name a public file may carry is `komira-ai/ucdk`.
fn other_org(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    lower
        .match_indices("komira")
        .find(|(i, _)| !lower[*i..].starts_with("komira-ai/ucdk"))
        .map(|(i, _)| lower[i..].chars().take(16).collect())
}

/// A tracker card id: `T` and three digits.
fn card_id(line: &str) -> Option<String> {
    tokens(line)
        .find(|t| t.len() == 4 && t.starts_with('T') && t[1..].bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_owned)
}

/// Every rule over every line of `text`; `subject` names the file or commit in the findings.
pub fn scan_text(subject: &str, text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    let lines: Vec<String> = text.lines().map(fold).collect();
    for (n, line) in lines.iter().enumerate() {
        for (rule, detect) in RULES {
            if let Some(found) = detect(line) {
                out.push(Finding::new(rule, format!("{subject}:{}", n + 1), found));
            }
        }
    }
    // A date or a phrase wrapped over two lines: the pair, reported at the first line, when
    // neither line alone is a finding.
    for (n, pair) in lines.windows(2).enumerate() {
        let joined = format!("{} {}", pair[0], pair[1]);
        for (rule, detect) in RULES.iter().filter(|(r, _)| WRAPPED.contains(r)) {
            if let Some(found) = detect(&joined)
                && detect(&pair[0]).is_none()
                && detect(&pair[1]).is_none()
            {
                out.push(Finding::new(rule, format!("{subject}:{}", n + 1), found));
            }
        }
    }
    out
}

/// The tree scan: `files` are (path, text) pairs.
pub fn check_tree(files: &[(String, String)]) -> Vec<Finding> {
    files
        .iter()
        .filter(|(path, _)| !EXEMPT.contains(&path.as_str()))
        .flat_map(|(path, text)| scan_text(path, text))
        .collect()
}

/// The one file of production code that reads the wall clock: the home of `SystemClock`.
pub const CLOCK_HOME: &str = "crates/ucdk-ledger/src/lease.rs";

/// What reads the wall clock, or a monotonic one.
const CLOCK_READS: [&str; 2] = ["SystemTime::now", "Instant::now"];

/// Whether `path` is production source: a `.rs` file under the `src` directory of a crate.
fn is_production_source(path: &str) -> bool {
    path.ends_with(".rs") && path.starts_with("crates/") && path.split('/').nth(2) == Some("src")
}

/// The wall-clock rule: `SystemTime::now` and `Instant::now` are read in production code only
/// in [`CLOCK_HOME`], so time reaches every other crate through the `Clock` it is given and a
/// test moves it by hand. Test code, which is a file outside `src` or what follows a
/// `#[cfg(test)]` line that gates a module of one (the unit-test module, last in its file),
/// and comment lines are not production code. A `#[cfg(test)]` on any other item hides
/// nothing after it.
pub fn check_wall_clock(files: &[(String, String)]) -> Vec<Finding> {
    files
        .iter()
        .filter(|(path, _)| is_production_source(path) && path != CLOCK_HOME)
        .flat_map(|(path, text)| {
            let lines: Vec<&str> = text.lines().collect();
            let test_module_at = lines.windows(2).position(|w| {
                w[0].trim() == "#[cfg(test)]"
                    && (w[1].trim_start().starts_with("mod ")
                        || w[1].trim_start().starts_with("pub mod "))
            });
            lines
                .into_iter()
                .enumerate()
                .take(test_module_at.unwrap_or(usize::MAX))
                .filter(|(_, line)| {
                    !line.trim_start().starts_with("//")
                        && CLOCK_READS.iter().any(|read| line.contains(read))
                })
                .map(move |(n, _)| {
                    Finding::new(
                        "wall-clock",
                        format!("{path}:{}", n + 1),
                        format!("the clock is read only in {CLOCK_HOME}; take a `Clock`"),
                    )
                })
        })
        .collect()
}

/// The name part of a trailer that carries an identity the lint lets through: the sign-off of
/// the commit's own author (`Signed-off-by: Name <author email>`), and the assistant's no-reply
/// co-author line. The address must be exactly that one; the name is still scanned by the rules.
fn trailer_name<'a>(line: &'a str, author_email: &str) -> Option<&'a str> {
    let (key, rest) = line.split_once(':')?;
    let email = match key.trim().to_ascii_lowercase().as_str() {
        "signed-off-by" => author_email.to_ascii_lowercase(),
        "co-authored-by" => "noreply@anthropic.com".to_owned(),
        _ => return None,
    };
    let rest = rest.trim();
    let open = rest.rfind('<')?;
    (rest[open..].to_ascii_lowercase() == format!("<{email}>")).then(|| rest[..open].trim())
}

/// The commit-message scan over the commits of a pull request, merge commits included.
pub fn check_commits(commits: &[CommitInfo]) -> Vec<Finding> {
    let mut out = Vec::new();
    for c in commits {
        let body: Vec<&str> = c
            .message
            .lines()
            .map(|l| trailer_name(l, &c.author_email).unwrap_or(l))
            .collect();
        let subject: String = c.id.chars().take(12).collect();
        out.extend(scan_text(&subject, &body.join("\n")));
    }
    out
}

/// The scan of the pull request title and body (a squash merge builds its message from them).
pub fn check_pr_text(event: &Event) -> Vec<Finding> {
    let mut out = scan_text("pull request title", &event.title);
    out.extend(scan_text("pull request body", &event.body));
    out
}
