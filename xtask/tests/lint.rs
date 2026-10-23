// SPDX-License-Identifier: Apache-2.0
//! The public-repository lint. The tree scan skips this file, because its samples are the
//! things the lint refuses.

mod common;

use xtask::dco::CommitInfo;
use xtask::event::Event;
use xtask::lint::{check_commits, check_pr_text, check_tree, scan_text};
use xtask::tree::read_tree;

fn join(parts: &[&str]) -> String {
    parts.concat()
}

fn rules(text: &str) -> Vec<&'static str> {
    scan_text("sample", text)
        .into_iter()
        .map(|f| f.rule)
        .collect()
}

fn file(path: &str, text: &str) -> (String, String) {
    (path.to_owned(), text.to_owned())
}

fn commit(author: &str, message: &str) -> CommitInfo {
    CommitInfo {
        id: "0123456789abcdef".to_owned(),
        parent_count: 1,
        author_email: author.to_owned(),
        message: message.to_owned(),
    }
}

#[test]
fn public_lint_rejects_old_date_and_home_path() {
    let dated = file(
        "docs/a.md",
        &format!("shipped {}-{:02}-{:02}\n", 2025, 10, 5),
    );
    let path = file(
        "docs/b.md",
        &format!("see {}\n", join(&["/ho", "me/someone/work"])),
    );
    let found: Vec<_> = check_tree(&[dated, path])
        .into_iter()
        .map(|f| (f.rule, f.subject))
        .collect();
    assert_eq!(
        found,
        [
            ("lint-date", "docs/a.md:1".to_owned()),
            ("lint-home-path", "docs/b.md:1".to_owned())
        ]
    );
}

#[test]
fn public_lint_rejects_internal_word_in_commit_message_and_pr_body() {
    let word = join(&["buc", "ket"]);
    let message =
        format!("Add storage\n\nMove the {word} code.\n\nSigned-off-by: A <a@example.org>\n");
    let found = check_commits(&[commit("a@example.org", &message)]);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(
        (found[0].rule, found[0].subject.as_str()),
        ("lint-word", "0123456789ab:3")
    );

    let event = Event {
        title: "Add storage".to_owned(),
        body: format!("About the {}.", word.to_uppercase()),
        ..Event::default()
    };
    let found = check_pr_text(&event);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].subject, "pull request body:1");
    let title = Event {
        title: word,
        ..Event::default()
    };
    assert_eq!(check_pr_text(&title)[0].subject, "pull request title:1");
}

#[test]
fn public_lint_accepts_clean_tree() {
    let tree = read_tree(&xtask::workspace_root()).expect("read the repository");
    for needed in [
        "README.md",
        "SECURITY.md",
        "CONTRIBUTING.md",
        "xtask/src/lint.rs",
    ] {
        assert!(
            tree.iter().any(|(p, _)| p == needed),
            "{needed} is in the tree"
        );
    }
    assert_eq!(check_tree(&tree), vec![]);
}

#[test]
fn lint_flags_dates_before_the_public_start_in_every_spelling() {
    let before = [
        format!("{}-{:02}-{:02}", 2025, 10, 5),
        format!("on {}/{:02}/{:02}.", 2026, 8, 31),
        format!("{}.{:02}.{:02}", 2024, 1, 2),
        join(&["Octo", "ber 5, 2025"]),
        join(&["5 Oc", "t 2025"]),
        join(&["in Ma", "rch 2026"]),
        join(&["2026 Au", "gust"]),
    ];
    for text in &before {
        assert_eq!(rules(text), ["lint-date"], "{text}");
    }
    let after = [
        format!("{}-{:02}-{:02}", 2026, 9, 1),
        format!("{}-{:02}-{:02}", 2027, 1, 15),
        join(&["Septem", "ber 2026"]),
        join(&["Jan", "uary 2027"]),
        "edition 2024".to_owned(),
        "version 0.20.2 and 1.2.3".to_owned(),
        "may 20 files change; Jan and Mar".to_owned(),
        "1234-5678".to_owned(),
        format!("{}-{:02}", 2025, 10),
    ];
    for text in &after {
        assert!(rules(text).is_empty(), "{text}");
    }
}

#[test]
fn lint_flags_home_paths() {
    for text in [
        join(&["/ho", "me/x"]),
        join(&["/Us", "ers/x"]),
        join(&["C:\\Us", "ers\\x"]),
    ] {
        assert_eq!(rules(&text), ["lint-home-path"], "{text}");
    }
    assert!(rules("~/.cargo and /usr/home-less").is_empty());
}

#[test]
fn lint_flags_hosts_and_addresses_outside_the_allow_list() {
    let bad = [
        "see https://builds.corp.invalid/x".to_owned(),
        "ssh://host.invalid:22/repo".to_owned(),
        format!("connect to {}.{}.{}.{}", 10, 0, 0, 1),
        join(&["node.example", ".ts.net"]),
        join(&["db.", "internal is up"]),
    ];
    for text in &bad {
        assert_eq!(rules(text), ["lint-host"], "{text}");
    }
    let good = [
        "https://github.com/komira-ai/ucdk/issues".to_owned(),
        "https://docs.github.com/en".to_owned(),
        "[x](https://crates.io/crates/serde) and https://developercertificate.org/".to_owned(),
        "http://localhost:8080 and https://x.example.org/".to_owned(),
        "https://doc.rust-lang.org/std/".to_owned(),
        "versions 1.2.3 and 1.2.3.4.5 and 300.1.1.1 and a.b.c.d".to_owned(),
        "the language and .lang files".to_owned(),
        "plain text, no scheme://".to_owned(),
        "file:///a/b and HTTPS://Github.COM/x".to_owned(),
    ];
    for text in &good {
        assert!(rules(text).is_empty(), "{text}");
    }
}

#[test]
fn lint_flags_addresses_account_ids_and_card_ids() {
    assert_eq!(
        rules(&join(&["mail me at bob@corp", ".invalid"])),
        ["lint-email"]
    );
    assert_eq!(
        rules(&join(&["id ", "123456789", "012 here"])),
        ["lint-account-id"]
    );
    assert_eq!(
        rules(&join(&["role ar", "n:aws:iam::x"])),
        ["lint-account-id"]
    );
    assert_eq!(rules(&join(&["see ", "T1", "23"])), ["lint-card-id"]);
    for text in [
        "a@example.org and a@sub.example.com",
        "@komira-ai/ucdk-maintainers owns it",
        "user@localhost and a@b",
        "11 digits 12345678901 and hex ab123456789012cd",
        "T12 and T1234 and Tab1 and 123",
    ] {
        assert!(rules(text).is_empty(), "{text}");
    }
}

#[test]
fn lint_flags_process_words_and_other_organisations() {
    for word in [
        join(&["mu", "tant"]),
        join(&["PLAN", "TED"]),
        join(&["MU", "T-3"]),
        join(&["it went", " red"]),
        join(&["Buc", "ket"]),
    ] {
        assert_eq!(rules(&word), ["lint-word"], "{word}");
    }
    assert_eq!(rules(&join(&["the Kom", "ira team"])), ["lint-org"]);
    assert_eq!(rules(&join(&["komira", "-ai/other"])), ["lint-org"]);
    assert!(
        rules(&join(&[
            "komira",
            "-ai/ucdk and KOMIRA",
            "-AI/UCDK-maintainers"
        ]))
        .is_empty()
    );
}

#[test]
fn commit_trailers_pass_only_for_the_authors_own_identity() {
    let own = join(&["adi@kom", "ira.com"]);
    let signed = format!(
        "Add x\n\nSigned-off-by: Adi Suresh <{own}>\nCo-Authored-By: Claude <noreply@anthropic.com>\n"
    );
    assert!(check_commits(&[commit(&own, &signed)]).is_empty());
    // The same lines are findings for another author, or with another co-author.
    assert!(!check_commits(&[commit("other@example.org", &signed)]).is_empty());
    let other = "Add x\n\nCo-Authored-By: Someone <someone@corp.invalid>\nno colon here\n";
    let found = check_commits(&[commit(&own, other)]);
    assert_eq!(
        found.iter().map(|f| f.rule).collect::<Vec<_>>(),
        ["lint-email"]
    );
    // A merge commit's message is not scanned.
    let mut merge = commit(&own, &join(&["Merge ", "buc", "ket"]));
    merge.parent_count = 2;
    assert!(check_commits(&[merge]).is_empty());
}

#[test]
fn tree_scan_skips_the_lint_source_and_the_licence_text() {
    let text = join(&["/ho", "me/x"]);
    let files = [
        file("xtask/src/lint.rs", &text),
        file("LICENSE", &text),
        file("other.rs", &text),
    ];
    let subjects: Vec<_> = check_tree(&files).into_iter().map(|f| f.subject).collect();
    assert_eq!(subjects, ["other.rs:1"]);
}
