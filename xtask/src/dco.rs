// SPDX-License-Identifier: Apache-2.0
//! DCO: every non-merge commit carries a `Signed-off-by:` trailer for its own author. Emails
//! compare case-insensitively, names are not compared.

use std::path::Path;

use crate::Finding;
use crate::cmd::{Cmd, Runner};

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub id: String,
    pub parent_count: usize,
    pub author_email: String,
    pub message: String,
}

/// The emails in the `Signed-off-by:` lines of the message's final paragraph.
fn signoff_emails(message: &str) -> Vec<String> {
    let body = message.trim_end();
    let last = body.rsplit("\n\n").next().unwrap_or("");
    last.lines()
        .filter_map(|line| {
            let (key, rest) = line.split_once(':')?;
            if !key.trim().eq_ignore_ascii_case("signed-off-by") {
                return None;
            }
            let rest = rest.trim_end();
            let open = rest.rfind('<')?;
            let email = rest[open + 1..].strip_suffix('>')?;
            Some(email.to_owned())
        })
        .collect()
}

pub fn check_dco(commits: &[CommitInfo]) -> Vec<Finding> {
    let mut out = Vec::new();
    for c in commits.iter().filter(|c| c.parent_count <= 1) {
        let emails = signoff_emails(&c.message);
        if emails.is_empty() {
            out.push(Finding::new(
                "dco-missing",
                &c.id,
                "no Signed-off-by trailer in the final paragraph (use git commit -s)",
            ));
        } else if !emails
            .iter()
            .any(|e| e.eq_ignore_ascii_case(&c.author_email))
        {
            out.push(Finding::new(
                "dco-other-person",
                &c.id,
                format!(
                    "Signed-off-by does not carry the author's email {}",
                    c.author_email
                ),
            ));
        }
    }
    out
}

/// Parses `git log -z --format=%H%x1f%P%x1f%ae%x1f%B` output: one NUL-terminated record per
/// commit. Git cannot store a NUL in a commit message, so a message cannot start a record of its
/// own; a message may hold U+001F, which stays in the last field. A record that is not
/// `<hex id> <hex parents> <email> <message>` is an error, never a skipped commit.
pub fn parse_log(text: &str) -> Result<Vec<CommitInfo>, String> {
    let is_hex = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit());
    let mut out = Vec::new();
    for record in text.split('\0').filter(|r| !r.trim().is_empty()) {
        let fields: Vec<&str> = record.splitn(4, '\u{1f}').collect();
        let [id, parents, email, message] = fields[..] else {
            return Err(format!("unreadable git log record: {record:?}"));
        };
        let id = id.trim();
        let parents: Vec<&str> = parents.split_whitespace().collect();
        if !is_hex(id) || !parents.iter().all(|p| is_hex(p)) {
            return Err(format!("unreadable git log record: {record:?}"));
        }
        out.push(CommitInfo {
            id: id.to_owned(),
            parent_count: parents.len(),
            author_email: email.to_owned(),
            message: message.to_owned(),
        });
    }
    Ok(out)
}

/// The commits in `base..HEAD`, oldest first.
pub fn commits_since(
    root: &Path,
    runner: &dyn Runner,
    base: &str,
) -> Result<Vec<CommitInfo>, String> {
    let range = format!("{base}..HEAD");
    let log = Cmd::new(
        "git",
        &[
            "log",
            "--reverse",
            "-z",
            "--format=%H%x1f%P%x1f%ae%x1f%B",
            &range,
        ],
        root,
    );
    runner.run(&log).and_then(|text| parse_log(&text))
}
