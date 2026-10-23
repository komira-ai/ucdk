// SPDX-License-Identifier: Apache-2.0
mod common;

use common::{Fake, git, scratch};
use xtask::cmd::System;
use xtask::dco::{CommitInfo, check_dco, commits_since, parse_log};
use xtask::event::{Event, commit_base, parse_event};

const A: &str = "a@example.org";

fn commit(parents: usize, author: &str, message: &str) -> CommitInfo {
    CommitInfo {
        id: "c1".to_owned(),
        parent_count: parents,
        author_email: author.to_owned(),
        message: message.to_owned(),
    }
}

fn rules(c: CommitInfo) -> Vec<&'static str> {
    check_dco(&[c]).into_iter().map(|f| f.rule).collect()
}

#[test]
fn dco_missing_signoff_fails() {
    assert_eq!(rules(commit(1, A, "Add x\n\nBody.\n")), ["dco-missing"]);
    assert_eq!(rules(commit(1, A, "Add x\n")), ["dco-missing"]);
    let late = "Add x\n\nSigned-off-by: A <a@example.org>\n\nText after the trailer.\n";
    assert_eq!(rules(commit(1, A, late)), ["dco-missing"]);
    let wrong_line = "Add x\n\nReviewed-by: A <a@example.org>\nSigned-off-by A <a@example.org>\n";
    assert_eq!(rules(commit(1, A, wrong_line)), ["dco-missing"]);
}

#[test]
fn dco_signoff_must_match_author() {
    // Same domain, different person: a domain-only comparison would accept this.
    let m = "Add x\n\nSigned-off-by: B <b@example.org>\n";
    assert_eq!(rules(commit(1, A, m)), ["dco-other-person"]);
    let upper = "Add x\n\nSigned-off-by: A <B@Example.ORG>\n";
    assert_eq!(rules(commit(1, A, upper)), ["dco-other-person"]);
}

#[test]
fn dco_accepts_a_matching_signoff_in_any_case_and_skips_merges() {
    let m = "Add x\n\nBody.\n\nSigned-off-by: A <a@example.org>\nCo-Authored-By: C <noreply@example.com>\n";
    assert!(rules(commit(1, A, m)).is_empty());
    let loose = "Add x\n\nsigned-off-BY: A <A@Example.ORG>\n";
    assert!(rules(commit(1, A, loose)).is_empty());
    assert!(rules(commit(1, "A@EXAMPLE.org", loose)).is_empty());
    assert!(rules(commit(2, A, "Merge branch 'x'\n")).is_empty());
    assert!(check_dco(&[]).is_empty());
}

#[test]
fn dco_empty_base_fails_on_pr() {
    let pr = Event {
        is_pull_request: true,
        ..Event::default()
    };
    for flag in [None, Some("")] {
        let finding = commit_base(&pr, flag).expect_err("a pull request needs a base");
        assert_eq!(finding.rule, "dco-no-base");
    }
}

#[test]
fn dco_reads_base_from_event_json() {
    let json = r#"{"pull_request":{"base":{"sha":"abc123"},"title":"T","body":"B"}}"#;
    let event = parse_event(json).expect("parse");
    assert_eq!(event.base_sha, "abc123");
    assert_eq!(commit_base(&event, None), Ok(Some("abc123".to_owned())));
    // A flag wins over the event.
    assert_eq!(commit_base(&event, Some("def")), Ok(Some("def".to_owned())));

    // And the base is used: only the commits after it are read, and a missing sign-off is found.
    let repo = scratch("dco-event");
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["commit", "--quiet", "--allow-empty", "-m", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]).trim().to_owned();
    git(
        &repo,
        &["commit", "--quiet", "--allow-empty", "-s", "-m", "signed"],
    );
    git(
        &repo,
        &["commit", "--quiet", "--allow-empty", "-m", "unsigned"],
    );
    let json = format!(r#"{{"pull_request":{{"base":{{"sha":"{base}"}}}}}}"#);
    let event = parse_event(&json).expect("parse");
    let base = commit_base(&event, None).expect("base").expect("some");
    let commits = commits_since(&repo, &System, &base).expect("log");
    let messages: Vec<&str> = commits
        .iter()
        .map(|c| c.message.lines().next().unwrap())
        .collect();
    assert_eq!(messages, ["signed", "unsigned"]);
    let found = check_dco(&commits);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].rule, "dco-missing");
}

#[test]
fn dco_unknown_base_is_an_error() {
    let repo = scratch("dco-bad-base");
    git(&repo, &["init", "--quiet"]);
    assert!(commits_since(&repo, &System, "no-such-commit").is_err());
}

#[test]
fn event_without_a_pull_request_has_no_base() {
    let push = parse_event(r#"{"ref":"refs/heads/main"}"#).expect("parse");
    assert!(!push.is_pull_request);
    assert_eq!(commit_base(&push, None), Ok(None));
    assert_eq!(commit_base(&push, Some("")), Ok(None));
    let null_body = parse_event(r#"{"pull_request":{"body":null}}"#).expect("parse");
    assert_eq!(null_body.body, "");
    assert!(parse_event("not json").is_err());
}

#[test]
fn log_output_is_split_into_commits() {
    let text = "\u{1e}h1\u{1f}p0\u{1f}a@example.org\u{1f}one\n\nbody\n\u{1e}h2\u{1f}p1 p2\u{1f}b@example.org\u{1f}merge\n";
    let commits = parse_log(text);
    assert_eq!(commits.len(), 2);
    assert_eq!((commits[0].id.as_str(), commits[0].parent_count), ("h1", 1));
    assert_eq!(commits[0].message, "one\n\nbody\n");
    assert_eq!(commits[1].parent_count, 2);
    assert!(parse_log("").is_empty());
    let fake = Fake::default();
    assert!(
        commits_since(&scratch("dco-fake"), &fake, "b")
            .expect("ok")
            .is_empty()
    );
    assert_eq!(fake.log.borrow().len(), 1);
}
