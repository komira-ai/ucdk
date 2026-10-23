// SPDX-License-Identifier: Apache-2.0
//! The public-repository lint: nothing in the tree, in a commit message or in the pull request
//! title and body may carry what a public repository must not: dates before the project's
//! public start, home paths, hostnames and addresses, account ids, e-mail addresses, other
//! organisations' names, names of people and teams, and internal process words. Text is folded
//! first (invisible characters dropped, look-alike letters, full-width forms, accents and
//! other scripts' digits turned into ASCII), so a rule cannot be dodged by spelling.
//!
//! This file and its tests are exempt from the tree scan (their patterns would match
//! themselves); every other file is scanned. Identities are allowed in commit metadata only
//! (author, committer and the matching trailers); `trailer_name` lets exactly those trailer
//! lines through, and their name part is scanned like any other text.

use crate::Finding;
use crate::dco::CommitInfo;
use crate::event::Event;

/// Files the tree scan skips: this file, its tests (whose samples would match), and the licence
/// text (which carries its own date).
const EXEMPT: [&str; 3] = ["xtask/src/lint.rs", "xtask/tests/lint.rs", "LICENSE"];

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

/// Month names in other languages, accents folded, with the month number.
const OTHER_MONTHS: &[(&str, u32)] = &[
    ("janvier", 1),
    ("fevrier", 2),
    ("mars", 3),
    ("avril", 4),
    ("mai", 5),
    ("juin", 6),
    ("juillet", 7),
    ("aout", 8),
    ("septembre", 9),
    ("octobre", 10),
    ("novembre", 11),
    ("decembre", 12),
    ("januar", 1),
    ("februar", 2),
    ("marz", 3),
    ("maerz", 3),
    ("juni", 6),
    ("juli", 7),
    ("oktober", 10),
    ("dezember", 12),
    ("enero", 1),
    ("febrero", 2),
    ("marzo", 3),
    ("abril", 4),
    ("mayo", 5),
    ("junio", 6),
    ("julio", 7),
    ("agosto", 8),
    ("septiembre", 9),
    ("setiembre", 9),
    ("noviembre", 11),
    ("diciembre", 12),
    ("gennaio", 1),
    ("febbraio", 2),
    ("aprile", 4),
    ("maggio", 5),
    ("giugno", 6),
    ("luglio", 7),
    ("settembre", 9),
    ("dicembre", 12),
    ("janeiro", 1),
    ("fevereiro", 2),
    ("marco", 3),
    ("maio", 5),
    ("junho", 6),
    ("julho", 7),
    ("setembro", 9),
    ("outubro", 10),
    ("novembro", 11),
];

const HOME_PATHS: [&str; 3] = ["/home/", "/Users/", "C:\\Users"];

/// Hosts a public file may link to; a subdomain of one is allowed too.
const ALLOWED_HOSTS: [&str; 11] = [
    "github.com",
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

/// Process words; built from pieces so this file does not contain them whole.
const WORDS: [&str; 5] = [
    concat!("mu", "tant"),
    concat!("plan", "ted"),
    concat!("mut", "-"),
    concat!("went", " red"),
    concat!("buc", "ket"),
];

/// Names of people and teams that a public file never carries (whole words, any case).
const NAMES: [&str; 7] = [
    concat!("c", "eo"),
    concat!("fl", "eet"),
    concat!("k", "bf"),
    concat!("mi", "ra"),
    concat!("spo", "olr"),
    concat!("ash", "win"),
    concat!("jer", "ry"),
];

/// A name with an underscore, matched as a substring.
const UNDERSCORE_NAME: &str = concat!("unity", "_cir");

type Detector = fn(&str) -> Option<String>;

const RULES: [(&str, Detector); 9] = [
    ("lint-date", old_date),
    ("lint-home-path", home_path),
    ("lint-host", host),
    ("lint-email", email),
    ("lint-account-id", account_id),
    ("lint-word", word),
    ("lint-name", name),
    ("lint-org", other_org),
    ("lint-card-id", card_id),
];

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

/// Letters of other scripts that look like a Latin letter, and that letter.
const LOOKALIKES: &[(char, char)] = &[
    ('а', 'a'),
    ('в', 'b'),
    ('е', 'e'),
    ('к', 'k'),
    ('м', 'm'),
    ('н', 'h'),
    ('о', 'o'),
    ('р', 'p'),
    ('с', 'c'),
    ('т', 't'),
    ('у', 'y'),
    ('х', 'x'),
    ('і', 'i'),
    ('ѕ', 's'),
    ('ј', 'j'),
    ('ԁ', 'd'),
    ('α', 'a'),
    ('ε', 'e'),
    ('ι', 'i'),
    ('ο', 'o'),
    ('ν', 'v'),
    ('ρ', 'p'),
    ('υ', 'u'),
];

/// Accented letters and the ASCII letters they fold to.
const ACCENTS: &[(&str, &str)] = &[
    ("àáâãäå", "a"),
    ("ç", "c"),
    ("èéêë", "e"),
    ("ìíîï", "i"),
    ("ñ", "n"),
    ("òóôõö", "o"),
    ("ùúûü", "u"),
    ("ýÿ", "y"),
    ("ß", "ss"),
    ("œ", "oe"),
    ("æ", "ae"),
];

/// First code point of each block of non-ASCII decimal digits (Arabic-Indic, Persian,
/// Devanagari, Bengali).
const DIGIT_BLOCKS: [u32; 4] = [0x660, 0x6f0, 0x966, 0x9e6];

fn other_digit(c: char) -> Option<char> {
    DIGIT_BLOCKS
        .iter()
        .find(|b| (**b..**b + 10).contains(&(c as u32)))
        .and_then(|b| char::from_digit(c as u32 - b, 10))
}

/// The line as a reader sees it, in ASCII where a letter or digit has an ASCII look-alike:
/// invisible characters dropped, full-width forms, other scripts' digits, look-alike letters
/// and accents folded. ASCII text is unchanged, so the rules see what they always saw.
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for c in line.chars().filter(|c| !invisible(*c)) {
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        for c in c.to_lowercase() {
            if let Some(digit) = other_digit(c) {
                out.push(digit);
            } else if let Some((_, latin)) = LOOKALIKES.iter().find(|(from, _)| *from == c) {
                out.push(*latin);
            } else if let Some((_, ascii)) = ACCENTS.iter().find(|(set, _)| set.contains(c)) {
                out.push_str(ascii);
            } else if ('\u{ff01}'..='\u{ff5e}').contains(&c) {
                out.push(char::from_u32(c as u32 - 0xfee0).unwrap_or(c));
            } else {
                out.push(c);
            }
        }
    }
    out
}

fn before_start(year: u32, month: u32) -> bool {
    (year, month) < START
}

fn old_date(line: &str) -> Option<String> {
    numeric_date(&glue_separators(line))
        .or_else(|| compact_date(line))
        .or_else(|| cjk_date(line))
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

/// Numbers joined by `-`, `/` or `.` that make a date (see `dated`).
fn numeric_date(line: &str) -> Option<String> {
    let chars = |c: char| !(c.is_ascii_digit() || SEPS.contains(&c));
    line.split(chars).find_map(|token| {
        let parts: Vec<&str> = token.split(SEPS).filter(|p| !p.is_empty()).collect();
        let short_year = parts.len() == 3;
        parts
            .windows(3)
            .any(|w| dated(w[0], w[1], w[2], short_year))
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

/// A four-digit year, `年`, a month number and `月`.
fn cjk_date(line: &str) -> Option<String> {
    line.match_indices('年').find_map(|(i, mark)| {
        let digits: Vec<char> = line[..i].chars().rev().take(4).collect();
        let year: String = digits.into_iter().rev().collect();
        let after = &line[i + mark.len()..];
        let month: String = after.chars().take_while(char::is_ascii_digit).collect();
        (after[month.len()..].starts_with('月') && dated(&year, &month, "1", false))
            .then(|| format!("{year}{mark}{month}月"))
    })
}

/// A month name (English full or three letters, or another language's) with a four-digit year
/// within two words of it.
fn month_date(line: &str) -> Option<String> {
    let words: Vec<&str> = tokens(line).collect();
    for (i, word) in words.iter().enumerate() {
        let lower = word.to_ascii_lowercase();
        let english = MONTHS
            .iter()
            .position(|m| lower == *m || lower == m[..3])
            .map(|p| p as u32 + 1);
        let other = || {
            OTHER_MONTHS
                .iter()
                .find(|(m, _)| lower == *m)
                .map(|(_, n)| *n)
        };
        let Some(month) = english.or_else(other) else {
            continue;
        };
        let near = &words[i.saturating_sub(2)..(i + 3).min(words.len())];
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

/// An internal name as a whole word, or one with an underscore anywhere.
fn name(line: &str) -> Option<String> {
    line.to_ascii_lowercase()
        .contains(UNDERSCORE_NAME)
        .then(|| UNDERSCORE_NAME.to_owned())
        .or_else(|| {
            tokens(line)
                .find(|t| NAMES.iter().any(|n| t.eq_ignore_ascii_case(n)))
                .map(str::to_owned)
        })
}

fn home_path(line: &str) -> Option<String> {
    HOME_PATHS
        .iter()
        .find(|p| line.contains(**p))
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

fn suffix(line: &str) -> Option<String> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || ".-".contains(c)))
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

fn word(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    WORDS
        .iter()
        .find(|w| lower.contains(**w))
        .map(|w| (*w).to_owned())
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
    for (n, line) in text.lines().enumerate() {
        let line = fold(line);
        for (rule, detect) in RULES {
            if let Some(found) = detect(&line) {
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

/// The commit-message scan over the commits of a pull request; merge commits are skipped.
pub fn check_commits(commits: &[CommitInfo]) -> Vec<Finding> {
    let mut out = Vec::new();
    for c in commits.iter().filter(|c| c.parent_count <= 1) {
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
