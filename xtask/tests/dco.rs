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
fn case_insensitive_key_and_email_ok() {
    let ok = "Add x\n\nsigned-off-BY: A <a@example.org>\n";
    assert!(rules(commit(1, A, ok)).is_empty());
    let upper = "Add x\n\nSigned-off-by: A <A@Example.ORG>\n";
    assert!(rules(commit(1, A, upper)).is_empty());
    assert!(rules(commit(1, "A@EXAMPLE.org", ok)).is_empty());
}

#[test]
fn different_local_part_still_red_despite_case() {
    let m = "Add x\n\nSigned-off-by: A <B@example.org>\n";
    assert_eq!(rules(commit(1, A, m)), ["dco-other-person"]);
}

mod plan {
    use xtask::check::dco_plan;

    #[test]
    fn pull_request_without_base_is_an_error() {
        for base in [None, Some("")] {
            let f = dco_plan(base, Some("pull_request")).expect_err("must refuse");
            assert_eq!(f.rule, "dco-no-base");
        }
    }

    #[test]
    fn other_events_without_base_skip() {
        assert_eq!(dco_plan(None, None), Ok(None));
        assert_eq!(dco_plan(Some(""), Some("push")), Ok(None));
    }

    #[test]
    fn a_base_always_runs_the_check() {
        assert_eq!(dco_plan(Some("abc"), Some("pull_request")), Ok(Some("abc")));
        assert_eq!(dco_plan(Some("abc"), None), Ok(Some("abc")));
    }
}
