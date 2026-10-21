// SPDX-License-Identifier: Apache-2.0
use xtask::dco::{CommitInfo, check_dco};

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

const A: &str = "a@example.org";

#[test]
fn missing_signoff_red() {
    assert_eq!(rules(commit(1, A, "Add x\n\nBody.\n")), ["dco-missing"]);
    assert_eq!(rules(commit(1, A, "Add x\n")), ["dco-missing"]);
}

#[test]
fn signoff_for_another_person_red() {
    // Same domain, different person: a domain-only comparison would accept this.
    let m = "Add x\n\nSigned-off-by: B <b@example.org>\n";
    assert_eq!(rules(commit(1, A, m)), ["dco-other-person"]);
}

#[test]
fn valid_signoff_green() {
    let m = "Add x\n\nBody.\n\nSigned-off-by: A <a@example.org>\nCo-Authored-By: Someone <noreply@example.com>\n";
    assert!(rules(commit(1, A, m)).is_empty());
}

#[test]
fn trailer_must_be_in_final_paragraph() {
    let m = "Add x\n\nSigned-off-by: A <a@example.org>\n\nMore text after the trailer.\n";
    assert_eq!(rules(commit(1, A, m)), ["dco-missing"]);
}

#[test]
fn merge_commits_skipped() {
    assert!(rules(commit(2, A, "Merge branch 'x'\n")).is_empty());
}

#[test]
fn empty_range_green() {
    assert!(check_dco(&[]).is_empty());
}

#[test]
fn case_insensitive_key_ok_but_exact_email_required() {
    let ok = "Add x\n\nsigned-off-BY: A <a@example.org>\n";
    assert!(rules(commit(1, A, ok)).is_empty());
    let wrong_case = "Add x\n\nSigned-off-by: A <A@example.org>\n";
    assert_eq!(rules(commit(1, A, wrong_case)), ["dco-other-person"]);
}
