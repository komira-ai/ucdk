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

/// Parses `git log --format=%x1e%H%x1f%P%x1f%ae%x1f%B` output.
pub fn parse_log(text: &str) -> Vec<CommitInfo> {
    text.split('\u{1e}')
        .filter(|r| !r.trim().is_empty())
        .map(|r| {
            let mut f = r.splitn(4, '\u{1f}');
            let id = f.next().unwrap_or("").trim().to_owned();
            let parent_count = f.next().unwrap_or("").split_whitespace().count();
            let author_email = f.next().unwrap_or("").to_owned();
            let message = f.next().unwrap_or("").to_owned();
            CommitInfo {
                id,
                parent_count,
                author_email,
                message,
            }
        })
        .collect()
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
            "--format=%x1e%H%x1f%P%x1f%ae%x1f%B",
            &range,
        ],
        root,
    );
    runner.run(&log).map(|text| parse_log(&text))
}
