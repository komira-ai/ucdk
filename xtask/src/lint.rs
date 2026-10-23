// SPDX-License-Identifier: Apache-2.0
//! The public-repository lint: nothing in the tree, in a commit message or in the pull request
//! title and body may carry what a public repository must not: dates before the project's
//! public start, home paths, hostnames and addresses, account ids, e-mail addresses, other
//! organisations' names and internal process words.
//!
//! This file and its tests are exempt from the tree scan (their patterns would match
//! themselves); every other file is scanned. Identities are allowed in commit metadata only (author, committer and the
//! matching trailers); `allowed_trailer` lets exactly those trailer lines through.

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

fn tokens(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
}

fn before_start(year: u32, month: u32) -> bool {
    (year, month) < START
}

fn old_date(line: &str) -> Option<String> {
    iso_date(line).or_else(|| month_date(line))
}

/// `YYYY-MM-DD` (also with `/` or `.`): a token of three numbers, the first of four digits.
fn iso_date(line: &str) -> Option<String> {
    let chars = |c: char| !(c.is_ascii_digit() || "-/.".contains(c));
    line.split(chars).find_map(|token| {
        let parts: Vec<&str> = token
            .trim_end_matches(['-', '/', '.'])
            .split(['-', '/', '.'])
            .collect();
        if parts.len() == 3 && parts[0].len() == 4 {
            let (year, month) = (parts[0].parse().ok()?, parts[1].parse().ok()?);
            return before_start(year, month).then(|| token.to_owned());
        }
        None
    })
}

fn year(word: &str) -> Option<u32> {
    Some(word)
        .filter(|w| w.len() == 4)
        .and_then(|w| w.parse().ok())
        .filter(|y| (1900..=2099).contains(y))
}

/// A month name (full or three letters) with a four-digit year within two words of it.
fn month_date(line: &str) -> Option<String> {
    let words: Vec<&str> = tokens(line).collect();
    for (i, word) in words.iter().enumerate() {
        let lower = word.to_ascii_lowercase();
        let Some(month) = MONTHS.iter().position(|m| lower == *m || lower == m[..3]) else {
            continue;
        };
        let near = &words[i.saturating_sub(2)..(i + 3).min(words.len())];
        if near
            .iter()
            .filter_map(|w| year(w))
            .any(|y| before_start(y, month as u32 + 1))
        {
            return Some(near.join(" "));
        }
    }
    None
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
        for (rule, detect) in RULES {
            if let Some(found) = detect(line) {
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

/// A trailer that carries an identity the lint lets through: the sign-off of the commit's own
/// author, and the assistant's no-reply co-author line.
fn allowed_trailer(line: &str, author_email: &str) -> bool {
    let Some((key, rest)) = line.split_once(':') else {
        return false;
    };
    let (key, rest) = (key.trim().to_ascii_lowercase(), rest.trim());
    let own = format!("<{}>", author_email.to_ascii_lowercase());
    (key == "signed-off-by" && rest.to_ascii_lowercase().ends_with(&own))
        || (key == "co-authored-by" && rest.ends_with("<noreply@anthropic.com>"))
}

/// The commit-message scan over the commits of a pull request; merge commits are skipped.
pub fn check_commits(commits: &[CommitInfo]) -> Vec<Finding> {
    let mut out = Vec::new();
    for c in commits.iter().filter(|c| c.parent_count <= 1) {
        let body: Vec<&str> = c
            .message
            .lines()
            .filter(|l| !allowed_trailer(l, &c.author_email))
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
