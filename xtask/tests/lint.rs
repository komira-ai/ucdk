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

#[test]
fn lint_flags_internal_names_as_whole_words_in_any_case() {
    for text in [
        join(&["the C", "EO said"]),
        join(&["fl", "eet"]),
        join(&["see k", "bf docs"]),
        join(&["Mi", "ra:"]),
        join(&["uni", "ty_", "cir"]),
        join(&["SPO", "OLR and others"]),
        join(&["ash", "win"]),
        join(&["jer", "ry"]),
    ] {
        assert_eq!(rules(&text), ["lint-name"], "{text}");
    }
    for text in [
        "a fleeting moment and a miracle",
        "kbfs and ceos and miras",
        "unity of purpose",
    ] {
        assert!(rules(text).is_empty(), "{text}");
    }
}

#[test]
fn lint_flags_dates_written_other_ways() {
    let before = [
        "2026 - 08 - 31".to_owned(),
        "item 5 - 2025-10-05".to_owned(),
        "end. 2025 / 10 / 05".to_owned(),
        join(&["2026", "0831"]),
        "08/31/2026".to_owned(),
        "31/08/2026".to_owned(),
        "31.8.2026".to_owned(),
        "31.8.26".to_owned(),
        "3/12/2025".to_owned(),
        "2026年8月31日".to_owned(),
        "\u{ff12}\u{ff10}\u{ff12}\u{ff15}-10-05".to_owned(),
        "\u{0662}\u{0660}\u{0662}\u{0665}-10-05".to_owned(),
        join(&["Ao", "ût 2026"]),
        join(&["31 ", "août 2026"]),
        join(&["Mä", "rz 2025"]),
        join(&["agos", "to de 2026"]),
        join(&["ju", "lho 2026"]),
        join(&["Aug", "\u{200b}ust 31 2026"]),
    ];
    for text in &before {
        assert_eq!(rules(text), ["lint-date"], "{text}");
    }
    let after = [
        "2026 - 09 - 01".to_owned(),
        "13/09/2026 and 09/13/2026".to_owned(),
        "15.9.26".to_owned(),
        "2026年9月1日".to_owned(),
        join(&["se", "ptembre 2026"]),
        "20260901 and 12345678 and 99999999999".to_owned(),
        "version 1.2.3.4.5 and 12.12.25.100.1 and 1.2.3".to_owned(),
        "3/12/2027 and 2026-13-45".to_owned(),
        "5 - 3 = 2 and 2026 年".to_owned(),
    ];
    for text in &after {
        assert!(rules(text).is_empty(), "{text}");
    }
}

#[test]
fn lint_sees_through_invisible_characters_and_look_alike_letters() {
    assert_eq!(rules(&join(&["mu", "\u{200b}", "tant"])), ["lint-word"]);
    assert_eq!(rules(&join(&["m", "\u{03c5}", "tant"])), ["lint-word"]);
    assert_eq!(rules(&join(&["mu", "\u{0301}", "tant"])), ["lint-word"]);
    assert_eq!(rules(&join(&["\u{ff4d}\u{ff55}tant"])), ["lint-word"]);
    assert_eq!(rules(&join(&["c", "\u{0435}", "o"])), ["lint-name"]);
    assert_eq!(
        rules(&join(&["/h", "\u{043e}", "me/x"])),
        ["lint-home-path"]
    );
    assert!(rules("na\u{ef}ve caf\u{e9} stra\u{df}e \u{153}uvre \u{e6}").is_empty());
}

#[test]
fn a_file_that_is_not_utf8_is_still_linted() {
    let dir = common::scratch("lint-lossy");
    let mut bytes = vec![0xff, b' '];
    bytes.extend_from_slice(format!("{}-{:02}-{:02}", 2025, 1, 1).as_bytes());
    std::fs::write(dir.join("notes.txt"), bytes).expect("write");
    let found = check_tree(&read_tree(&dir).expect("read"));
    assert_eq!(
        found.iter().map(|f| f.rule).collect::<Vec<_>>(),
        ["lint-date"]
    );
}

#[test]
fn a_trailer_is_not_a_way_to_carry_text() {
    let own = join(&["adi@kom", "ira.com"]);
    let date = format!("{}-{:02}-{:02}", 2025, 1, 1);
    let name = join(&["mi", "ra"]);
    let sign = format!("Add x\n\nSigned-off-by: leaked on {date} {name} <{own}>\n");
    let found: Vec<_> = check_commits(&[commit(&own, &sign)])
        .iter()
        .map(|f| f.rule)
        .collect();
    assert_eq!(found, ["lint-date", "lint-name"]);
    let host = format!(
        "Co-Authored-By: host {}.{}.{}.{} <noreply@anthropic.com>\n",
        10, 0, 0, 1
    );
    let found: Vec<_> = check_commits(&[commit(&own, &format!("Add x\n\n{host}"))])
        .iter()
        .map(|f| f.rule)
        .collect();
    assert_eq!(found, ["lint-host"]);
    // Another address, another key, a missing address: the whole line is scanned.
    for line in [
        format!("Signed-off-by: Adi <someone@example.org> {own}"),
        format!("Reviewed-by: Adi <{own}>"),
        format!("Signed-off-by: Adi {own}"),
    ] {
        assert!(
            !check_commits(&[commit(&own, &format!("x\n\n{line}\n"))]).is_empty(),
            "{line}"
        );
    }
}
