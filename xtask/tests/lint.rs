// SPDX-License-Identifier: Apache-2.0
//! The public-repository lint. The tree scan skips this file, because its samples are the
//! things the lint refuses.

mod common;

use xtask::dco::CommitInfo;
use xtask::event::Event;
use xtask::lint::{
    CLOCK_HOME, check_commits, check_pr_text, check_tree, check_wall_clock, scan_text,
};
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
fn wall_clock_is_read_only_in_system_clock() {
    let read = join(&["let at = std::time::System", "Time::now();\n"]);
    let monotonic = join(&["let at = std::time::Inst", "ant::now();\n"]);
    let source = |path: &str, text: &str| file(path, text);
    let rules = |files: &[(String, String)]| -> Vec<(String, &'static str)> {
        check_wall_clock(files)
            .into_iter()
            .map(|f| (f.subject, f.rule))
            .collect()
    };
    // Production code of any crate but the home of the clock is refused, either reader.
    let refused = rules(&[
        source(
            "crates/ucdk-release/src/a.rs",
            &format!("fn f() {{\n{read}}}\n"),
        ),
        source("crates/ucdk-cli/src/b/c.rs", &monotonic),
    ]);
    assert_eq!(
        refused,
        [
            ("crates/ucdk-release/src/a.rs:2".to_owned(), "wall-clock"),
            ("crates/ucdk-cli/src/b/c.rs:1".to_owned(), "wall-clock"),
        ]
    );
    // The home of the clock, tests, other languages, other directories and comments are not.
    let allowed = rules(&[
        source(CLOCK_HOME, &read),
        source("crates/ucdk-release/tests/a.rs", &read),
        source("crates/ucdk-release/src/a.md", &read),
        source("crates/ucdk-release/build.rs", &read),
        source("xtask/src/a.rs", &read),
        source("crates/ucdk-release/src/doc.rs", &format!("// {read}")),
        source(
            "crates/ucdk-release/src/unit.rs",
            &format!("fn f() {{}}\n#[cfg(test)]\nmod t {{\n{read}}}\n"),
        ),
    ]);
    assert!(allowed.is_empty(), "{allowed:?}");
    // The repository itself holds the rule.
    let tree = read_tree(&xtask::workspace_root()).unwrap();
    assert_eq!(check_wall_clock(&tree), []);
    assert!(tree.iter().any(|(path, _)| path == CLOCK_HOME));
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
        "version 0.20.2 and 1.2.3".to_owned(),
        "may 20 files change; Jan and Mar".to_owned(),
        join(&["Jan", "uary 2027"]),
        "edition 2024".to_owned(),
        "1234-5678".to_owned(),
        format!("{}-{:02}", 2026, 9),
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
        join(&["see https:", "//builds.corp.invalid/x"]),
        join(&["ssh:", "//host.invalid:22/repo"]),
        format!("connect to {}.{}.{}.{}", 10, 0, 0, 1),
        join(&["node.example.ts", ".net"]),
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
        "https://nodejs.org/dist/v26.12.1/node-v26.12.1-linux-x64.tar.gz".to_owned(),
        "versions 1.2.3 and 1.2.3.4.5 and 300.1.1.1 and a.b.c.d".to_owned(),
        "the language and .lang files".to_owned(),
        "plain text, no scheme://".to_owned(),
        "file:///a/b and HTTPS://Github.COM/x".to_owned(),
        // An attribute of an object, as in generated code, is not a host.
        join(&["if self.", "internal is not None:"]),
        join(&["if (this.", "internal !== undefined) {"]),
        join(&["this.internal = init.", "internal;"]),
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
        join(&["it went", " r", "ed"]),
        join(&["Buc", "ket"]),
    ] {
        assert_eq!(rules(&word), ["lint-word"], "{word}");
    }
    assert_eq!(rules(&join(&["the Kom", "ira team"])), ["lint-org"]);
    assert_eq!(rules(&join(&["kom", "ira-ai/other"])), ["lint-org"]);
    assert!(
        rules(&join(&[
            "kom",
            "ira-ai/ucdk and KOM",
            "IRA-AI/UCDK-maintainers"
        ]))
        .is_empty()
    );
}

#[test]
fn commit_trailers_pass_only_for_the_authors_own_identity() {
    let own = join(&["author@corp", ".invalid"]);
    let noreply = join(&["noreply@anthro", "pic.com"]);
    let signed =
        format!("Add x\n\nSigned-off-by: A Person <{own}>\nCo-Authored-By: Claude <{noreply}>\n");
    assert!(check_commits(&[commit(&own, &signed)]).is_empty());
    // The same lines are findings for another author, or with another co-author.
    assert!(!check_commits(&[commit("other@example.org", &signed)]).is_empty());
    let other = format!(
        "Add x\n\nCo-Authored-By: Someone <{}>\nno colon here\n",
        join(&["someone@corp", ".invalid"])
    );
    let found = check_commits(&[commit(&own, &other)]);
    assert_eq!(
        found.iter().map(|f| f.rule).collect::<Vec<_>>(),
        ["lint-email"]
    );
    // A merge commit's message is scanned like any other.
    let mut merge = commit(&own, &join(&["Merge ", "buc", "ket"]));
    merge.parent_count = 2;
    assert_eq!(check_commits(&[merge])[0].rule, "lint-word");
}

#[test]
fn tree_scan_skips_only_the_lint_source_and_the_licence_text() {
    let text = join(&["/ho", "me/x"]);
    let files = [
        file("xtask/src/lint.rs", &text),
        file("LICENSE", &text),
        file("xtask/tests/lint.rs", &text),
        file("other.rs", &text),
    ];
    let subjects: Vec<_> = check_tree(&files).into_iter().map(|f| f.subject).collect();
    assert_eq!(subjects, ["xtask/tests/lint.rs:1", "other.rs:1"]);
}

#[test]
fn lint_flags_dates_written_other_ways() {
    let before = [
        join(&["Aug", "\u{200b}ust 31 2026"]),
        format!("{} - {:02} - {:02}", 2026, 8, 31),
        format!("item 5 - {}-{:02}-{:02}", 2025, 10, 5),
        format!("end. {} / {:02} / {:02}", 2025, 10, 5),
        format!("{}{:02}{:02}", 2026, 8, 31),
        format!("{:02}/{:02}/{}", 8, 31, 2026),
        format!("{:02}/{:02}/{}", 31, 8, 2026),
        format!("{}.{}.{}", 31, 8, 2026),
        format!("{}.{}.{}", 31, 8, 26),
        format!("{}/{}/{}", 3, 12, 2025),
        // The year and month alone.
        format!("{}-{:02}", 2026, 8),
        format!("{}-{:02}", 2025, 10),
        format!("{:02}/{}", 8, 2026),
        format!("see {}/{}.", 1, 2025),
        // A year-first date with a two-digit year.
        format!("{}-{:02}-{:02}", 26, 8, 31),
        // Hyphens that are not the ASCII one.
        format!("{}\u{2011}{:02}\u{2011}{:02}", 2026, 8, 31),
        format!("{}\u{2013}{:02}\u{2013}{:02}", 2026, 8, 31),
        format!("{}\u{2212}{:02}", 2026, 8),
        // The month names: a short form of September, and the year a few words away.
        join(&["S", "ept 2025"]),
        join(&["S", "ept. 3, 2025"]),
        join(&["in Ma", "rch of this 2026"]),
    ];
    for text in &before {
        assert_eq!(rules(text), ["lint-date"], "{text}");
    }
    let after = [
        format!("{} - {:02} - {:02}", 2026, 9, 1),
        "13/09/2026 and 09/13/2026".to_owned(),
        "15.9.26".to_owned(),
        "20260901 and 12345678 and 99999999999".to_owned(),
        "version 1.2.3.4.5 and 12.12.25.100.1 and 1.2.3".to_owned(),
        "3/12/2027 and 2026-13-45".to_owned(),
        "5 - 3 = 2 and 2026".to_owned(),
        // A year and a month from the start on, a version, an out-of-range month or year.
        format!("{}-{:02} and {}/{}", 2026, 9, 9, 2026),
        "1.2024 and 2024.1 and 2025-13 and 1850-05".to_owned(),
        format!("{}/{:02}/{:02}", 26, 8, 31),
    ];
    for text in &after {
        assert!(rules(text).is_empty(), "{text}");
    }
}

#[test]
fn a_date_or_phrase_wrapped_over_two_lines_is_flagged_at_its_first_line() {
    let month = join(&["Au", "gust"]);
    let wrapped = format!("notes from {month}\n2026 were kept\nand more\n");
    let found = scan_text("sample", &wrapped);
    assert_eq!(
        found
            .iter()
            .map(|f| (f.rule, f.subject.as_str()))
            .collect::<Vec<_>>(),
        [("lint-date", "sample:1")]
    );
    let split = format!("a run that went\n{} after all", join(&["r", "ed"]));
    let found = scan_text("sample", &split);
    assert_eq!(
        found
            .iter()
            .map(|f| (f.rule, f.subject.as_str()))
            .collect::<Vec<_>>(),
        [("lint-word", "sample:1")]
    );
    // A line that is a finding on its own is reported once, not again with its neighbour.
    let own = format!("{}-{:02}-{:02}\nplain\n", 2025, 1, 1);
    assert_eq!(scan_text("sample", &own).len(), 1);
    // Neighbours that make nothing together are not findings.
    assert!(rules("it went\nwell\nand then\nred apples\n").is_empty());
}

#[test]
fn process_words_match_whole_words_and_any_spacing() {
    let phrase = |gap: &str| format!("it went{gap}{}", join(&["r", "ed"]));
    for text in [
        phrase("  "),
        phrase("\t"),
        phrase(", "),
        join(&["the mu", "tants and buc", "kets"]),
        join(&["MU", "T -3"]),
        join(&["(mu", "t-7)"]),
    ] {
        assert_eq!(rules(&text), ["lint-word"], "{text}");
    }
    for text in [
        join(&["a sup", "plant", "ed idea and an im", "plant", "ed one"]).as_str(),
        "a mutation and a permutation",
        "he went to read; the red fox",
        "fn f(x: &mut Vec<u8>) and mut x",
        "summut-free and demut -",
    ] {
        assert!(rules(text).is_empty(), "{text}");
    }
}

#[test]
fn lint_sees_through_invisible_characters_and_case() {
    assert_eq!(rules(&join(&["mu", "\u{200b}", "tant"])), ["lint-word"]);
    assert_eq!(rules(&join(&["mu", "\u{0301}", "tant"])), ["lint-word"]);
    assert_eq!(rules(&join(&["/h", "\u{ad}", "ome/x"])), ["lint-home-path"]);
    assert_eq!(rules(&join(&["/HO", "ME/x"])), ["lint-home-path"]);
    assert_eq!(rules(&join(&["c:\\us", "ers\\x"])), ["lint-home-path"]);
    assert!(rules("na\u{ef}ve caf\u{e9} stra\u{df}e").is_empty());
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
    let own = join(&["author@corp", ".invalid"]);
    let noreply = join(&["noreply@anthro", "pic.com"]);
    let date = format!("{}-{:02}-{:02}", 2025, 1, 1);
    let name = join(&["buc", "ket"]);
    let sign = format!("Add x\n\nSigned-off-by: leaked on {date} {name} <{own}>\n");
    let found: Vec<_> = check_commits(&[commit(&own, &sign)])
        .iter()
        .map(|f| f.rule)
        .collect();
    assert_eq!(found, ["lint-date", "lint-word"]);
    let host = format!(
        "Co-Authored-By: host {}.{}.{}.{} <{noreply}>\n",
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

#[test]
fn pr_body_attribution_link_is_allowed() {
    let attribution = "🤖 Generated with [Claude Code](https://claude.com/claude-code)";
    let event = Event {
        title: "Add storage".to_owned(),
        body: format!("Adds storage.\n\n{attribution}"),
        ..Event::default()
    };
    assert_eq!(check_pr_text(&event), vec![]);

    let other = join(&["https:", "//tools.vendor", ".invalid/page"]);
    let event = Event {
        body: format!("Adds storage.\n\n{attribution}\n{other}"),
        ..Event::default()
    };
    let found = check_pr_text(&event);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].subject, "pull request body:4");
}
