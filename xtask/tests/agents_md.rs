// SPDX-License-Identifier: Apache-2.0
//! `AGENTS.md` (the rules every change is written and reviewed against) and `CLAUDE.md`
//! (which imports it). The checks are functions of the texts, so each is tested on edited
//! texts as well as on the real files.

use std::fs;

use xtask::agents::{
    AGENTS, AgentsError, BEGIN, CLAUDE, COPY, Core, END, HASH, IMPORT, core, hash_line,
    hash_matches, imports_agents, rule_ids, same_core, withheld,
};

/// A file of the repository, by its path from the root.
fn read(rel: &str) -> String {
    fs::read_to_string(xtask::workspace_root().join(rel)).expect(rel)
}

/// Catches the core of the real `AGENTS.md` drifting from its copy.
#[test]
fn agents_md_core_is_the_copy() {
    let agents = read(AGENTS);
    let found = core(&agents).expect("AGENTS.md has a shared core");
    assert_eq!(same_core(&found, &read(COPY)), Ok(()));
}

/// Catches the copy edited without its hash.
#[test]
fn the_copy_is_the_one_its_hash_names() {
    assert_eq!(hash_matches(read(COPY).as_bytes(), &read(HASH)), Ok(()));
}

/// Catches `CLAUDE.md` no longer importing `AGENTS.md`.
#[test]
fn claude_md_imports_agents_md() {
    assert!(
        imports_agents(&read(CLAUDE)),
        "{CLAUDE} has no line `{IMPORT}`"
    );
}

/// Catches the core emptied or cut short together with its copy and hash: the copy holds the
/// five headings of the shared core and the first rule of each.
#[test]
fn the_real_core_has_every_section_of_the_shared_core() {
    let copy = read(COPY);
    let lines: Vec<&str> = copy.lines().collect();
    for heading in [
        "### Tests and coverage",
        "### Design and change shape",
        "### CI and workflows",
        "### PR text and commits",
        "### Secrets and configuration",
    ] {
        assert!(
            lines.contains(&heading),
            "{COPY} has no heading `{heading}`"
        );
    }
    let ids = rule_ids(&copy);
    for id in ["T1", "B8", "B9", "P1", "S1"] {
        assert!(ids.contains(&id), "{COPY} has no rule {id}");
    }
}

/// Catches a rule that is the reviewer's alone, or meant only for the project's own sessions,
/// written into this public file: `AGENTS.md` states rules, and none of them is withheld.
#[test]
fn agents_md_states_no_withheld_rule() {
    let agents = read(AGENTS);
    let ids = rule_ids(&agents);
    assert!(ids.contains(&"T1") && ids.contains(&"R10"), "{ids:?}");
    let stated: Vec<&str> = ids.into_iter().filter(|id| withheld(id)).collect();
    assert!(stated.is_empty(), "{AGENTS} states {stated:?}");
}

/// Catches a rule id missed in a list form other than `- ` at the margin, or read out of
/// prose: the first word of a list item of any form, and nothing else, is a rule id.
#[test]
fn rule_ids_are_the_first_word_of_a_list_item() {
    let text = "\
- F1 BLOCK. dash
  - H2 indented
* G3 star
+ H4 plus
1. H5 numbered
12) S5 numbered with a parenthesis
- **H6** bold
- `H7`: in code, with a colon
- T19. with a dot
-\tH9 dash and a tab
3.\tH3 number and a tab
7)\tT7 parenthesis and a tab
See F1 and H2 in prose.
- Follow F1 first.
-F1 no space after the bullet
1.H1 no space after the number
-\u{a0}H1 a no-break space is no separator
1:\tH1 a colon is no number marker
- f1 lower case
- F no digits
- 1F digits first
- F1a letters after
. H8 a dot is no bullet
-
";
    assert_eq!(
        rule_ids(text),
        [
            "F1", "H2", "G3", "H4", "H5", "S5", "H6", "H7", "T19", "H9", "H3", "T7"
        ]
    );
    assert_eq!(rule_ids(""), Vec::<&str>::new());
    // A tab-separated withheld rule is found, and refused, like a space-separated one.
    let tabbed = rule_ids("-\tH1 BLOCK. x\n1.\tF1 x\n-\tT1 BLOCK. y\n");
    assert_eq!(tabbed, ["H1", "F1", "T1"]);
    assert_eq!(
        tabbed
            .into_iter()
            .filter(|id| withheld(id))
            .collect::<Vec<_>>(),
        ["H1", "F1"]
    );
}

/// Catches a withheld rule let through, or a stated one withheld: F, G and H rules and S5 are
/// withheld, every other id is not.
#[test]
fn withheld_rules_are_f_g_h_and_s5() {
    for id in ["F1", "F2", "G1", "G5", "H1", "H8", "S5"] {
        assert!(withheld(id), "{id}");
    }
    for id in [
        "S1", "S4", "S50", "T1", "R10", "B9", "P4", "M1", "F", "", "s5",
    ] {
        assert!(!withheld(id), "{id}");
    }
}

fn agents_with(core_text: &str) -> String {
    format!("# AGENTS.md\n\nintro\n\n{BEGIN}\n{core_text}{END}\n\n## 2. Rust\n")
}

/// A core that starts on line 6 of its `AGENTS.md`.
fn at(text: &str) -> Core<'_> {
    Core { text, lines: 6..10 }
}

const CORE: &str = "## 1. Shared core\n\n- T1 BLOCK. one\n- T2 SHOULD. two\n";

fn found(text: &str, lines: std::ops::Range<usize>) -> Result<Core<'_>, AgentsError> {
    Ok(Core { text, lines })
}

/// Catches a core read from the wrong place: the text between the markers, numbered from the
/// line after the begin marker.
#[test]
fn core_is_the_text_between_the_marker_lines() {
    assert_eq!(core(&agents_with(CORE)), found(CORE, 6..10));
    // An empty core: the markers on adjacent lines.
    assert_eq!(core(&agents_with("")), found("", 6..6));
    // The end marker as the last line, with no newline after it.
    assert_eq!(core(&format!("{BEGIN}\nx\n{END}")), found("x\n", 2..3));
}

/// Catches byte offsets taken for character counts: with characters of two, three and four
/// bytes before the begin marker and inside the core, the core is the exact text and lines,
/// and a later differing line is named by its line, not its offset.
#[test]
fn a_core_after_and_with_multibyte_text_is_read_by_bytes_and_numbered_by_lines() {
    let inner = "## Kern é\n- T1 BLOCK. ≥ 99% ✓\n- T2 SHOULD. 🦀 two\n";
    let agents = format!("# Régles ✓ 🦀\nñ\n{BEGIN}\n{inner}{END}\nafter ü\n");
    let got = core(&agents).expect("core");
    assert_eq!(
        got,
        Core {
            text: inner,
            lines: 4..7
        }
    );
    let copy = inner.replace("two", "zwei");
    let err = same_core(&got, &copy).expect_err("differs");
    assert_eq!(
        err,
        AgentsError::CoreDiffers {
            agents_line: 6,
            copy_line: 3,
            agents: Some("- T2 SHOULD. 🦀 two\n".to_owned()),
            copy: Some("- T2 SHOULD. 🦀 zwei\n".to_owned()),
        }
    );
    assert_eq!(
        err.to_string(),
        format!(
            "AGENTS.md:6: the shared core differs from {COPY}:3: AGENTS.md has \"- T2 SHOULD. 🦀 two\\n\", the copy has \"- T2 SHOULD. 🦀 zwei\\n\""
        )
    );
}

fn count(begins: usize, ends: usize) -> Result<Core<'static>, AgentsError> {
    Err(AgentsError::MarkerCount { begins, ends })
}

/// Catches a marker that is missing, repeated, out of order, or not a line of its own, at
/// either end; each refusal is its variant and its message.
#[test]
fn markers_that_cannot_delimit_the_core_are_refused() {
    let times =
        |m: &str, n: usize| format!("AGENTS.md: the line `{m}` is there {n} times, not once");
    let message = |text: &str| core(text).expect_err("refused").to_string();
    assert_eq!(core(""), count(0, 0));
    assert_eq!(
        message(""),
        format!("{}\n{}", times(BEGIN, 0), times(END, 0))
    );
    assert_eq!(core("no markers\n"), count(0, 0));
    assert_eq!(core(&format!("{BEGIN}\nx\n")), count(1, 0));
    assert_eq!(message(&format!("{BEGIN}\nx\n")), times(END, 0));
    assert_eq!(core(&format!("x\n{END}\n")), count(0, 1));
    assert_eq!(message(&format!("x\n{END}\n")), times(BEGIN, 0));
    let twice_begin = format!("{BEGIN}\n{BEGIN}\nx\n{END}\n");
    assert_eq!(core(&twice_begin), count(2, 1));
    assert_eq!(message(&twice_begin), times(BEGIN, 2));
    let twice_end = format!("{BEGIN}\nx\n{END}\n{END}\n");
    assert_eq!(core(&twice_end), count(1, 2));
    assert_eq!(message(&twice_end), times(END, 2));
    let both = format!("{BEGIN}\n{BEGIN}\n{END}\n{END}\n");
    assert_eq!(core(&both), count(2, 2));
    assert_eq!(
        message(&both),
        format!("{}\n{}", times(BEGIN, 2), times(END, 2))
    );
    let reversed = format!("{END}\nx\n{BEGIN}\n");
    assert_eq!(
        core(&reversed),
        Err(AgentsError::MarkersOutOfOrder { begin: 3, end: 1 })
    );
    assert_eq!(
        message(&reversed),
        "AGENTS.md:1: the end of the shared core comes before its begin (line 3)"
    );
    // A marker with text around it, indented, or with a carriage return is not a marker line,
    // the begin and the end alike.
    for marker in [BEGIN, END] {
        for bad in [
            format!(" {marker}"),
            format!("\t{marker}"),
            format!("{marker} "),
            format!("{marker}\r"),
            format!("see {marker}"),
            format!("{marker} here"),
        ] {
            let (text, want, why) = if marker == BEGIN {
                (format!("{bad}\nx\n{END}\n"), count(0, 1), times(BEGIN, 0))
            } else {
                (format!("{BEGIN}\nx\n{bad}\n"), count(1, 0), times(END, 0))
            };
            assert_eq!(core(&text), want, "{bad:?}");
            assert_eq!(message(&text), why, "{bad:?}");
        }
    }
}

/// Catches an edited core: a changed, added or removed line, a carriage return, or a
/// difference only in the final newline, each named by its line in AGENTS.md and in the copy.
#[test]
fn a_core_that_differs_from_the_copy_names_the_first_differing_line() {
    let differs = |agents_line: usize, copy_line: usize, got: Option<&str>, want: Option<&str>| {
        AgentsError::CoreDiffers {
            agents_line,
            copy_line,
            agents: got.map(str::to_owned),
            copy: want.map(str::to_owned),
        }
    };
    let refused = |text: &str, copy: &str| same_core(&at(text), copy).expect_err("differs");
    assert_eq!(same_core(&at(CORE), CORE), Ok(()));
    let changed = CORE.replace("two", "2");
    assert_eq!(
        refused(&changed, CORE),
        differs(9, 4, Some("- T2 SHOULD. 2\n"), Some("- T2 SHOULD. two\n"))
    );
    assert_eq!(
        refused(&changed, CORE).to_string(),
        format!(
            "AGENTS.md:9: the shared core differs from {COPY}:4: AGENTS.md has \"- T2 SHOULD. 2\\n\", the copy has \"- T2 SHOULD. two\\n\""
        )
    );
    let added = format!("{CORE}- T3 BLOCK. three\n");
    assert_eq!(
        refused(&added, CORE),
        differs(10, 5, Some("- T3 BLOCK. three\n"), None)
    );
    assert_eq!(
        refused(&added, CORE).to_string(),
        format!(
            "AGENTS.md:10: the shared core differs from {COPY}:5: AGENTS.md has \"- T3 BLOCK. three\\n\", the copy has the end of the text"
        )
    );
    assert_eq!(
        refused(CORE, &added),
        differs(10, 5, None, Some("- T3 BLOCK. three\n"))
    );
    assert_eq!(
        refused(CORE, &added).to_string(),
        format!(
            "AGENTS.md:10: the shared core differs from {COPY}:5: AGENTS.md has the end of the text, the copy has \"- T3 BLOCK. three\\n\""
        )
    );
    let crlf = CORE.replacen('\n', "\r\n", 1);
    assert_eq!(
        refused(&crlf, CORE),
        differs(
            6,
            1,
            Some("## 1. Shared core\r\n"),
            Some("## 1. Shared core\n")
        )
    );
    assert_eq!(
        refused(CORE.trim_end_matches('\n'), CORE),
        differs(9, 4, Some("- T2 SHOULD. two"), Some("- T2 SHOULD. two\n"))
    );
    // A core emptied in AGENTS.md alone.
    assert_eq!(
        refused("", CORE),
        differs(6, 1, None, Some("## 1. Shared core\n"))
    );
}

/// Catches a copy edited without its hash, and a hash file in any form but the one
/// `sha256sum` writes.
#[test]
fn the_hash_file_is_the_copys_sha256sum_line() {
    let copy = CORE.as_bytes();
    let good = hash_line(copy);
    // The digest of the sample, worked out independently of the code under test.
    assert_eq!(
        hash_line(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  shared_core.md\n"
    );
    assert_eq!(hash_matches(copy, &good), Ok(()));
    let hex = good.split(' ').next().expect("digest");
    for bad in [
        // The copy edited: the hash of other text.
        hash_line(b"x"),
        format!("{}  shared_core.md\n", hex.to_uppercase()),
        format!("{hex}  shared_core.md"),
        format!("{hex}  shared_core.md\n\n"),
        format!("{hex}  shared_core.md\r\n"),
        format!("{hex} shared_core.md\n"),
        format!("{hex}  docs/agents/shared_core.md\n"),
        format!("{hex}\n"),
        String::new(),
    ] {
        let err = hash_matches(copy, &bad).expect_err("refused");
        assert_eq!(
            err,
            AgentsError::HashMismatch {
                found: bad.clone(),
                want: good.clone()
            },
            "{bad:?}"
        );
        assert_eq!(
            err.to_string(),
            format!(
                "{HASH} is {bad:?}, not {good:?} (run `sha256sum shared_core.md` in docs/agents)"
            ),
            "{bad:?}"
        );
    }
    // It is an error a caller can pass on as one.
    let boxed: Box<dyn std::error::Error> = Box::new(hash_matches(copy, "").expect_err("refused"));
    assert!(boxed.to_string().starts_with(HASH));
}

/// A sample of `CLAUDE.md` with `{i}` standing for the import line (and `{i_lower}` for it in
/// lower case), so that no line of this file reads as an address.
fn with_import(sample: &str) -> String {
    sample
        .replace("{i_lower}", &IMPORT.to_lowercase())
        .replace("{i}", IMPORT)
}

/// Catches CLAUDE.md losing its import: only a line that is exactly `@AGENTS.md`, outside a
/// fenced code block, counts.
#[test]
fn claude_md_imports_agents_md_only_by_a_line_of_its_own_outside_a_fence() {
    for good in [
        "{i}\n",
        "{i}",
        "# Notes\n\nucdk specifics\n\n{i}\n",
        "```\ncode\n```\n{i}\n",
        "~~~~\n```\n~~~~\n{i}\n",
        "````\n```\n````\n{i}\n",
        "```\n```  \n{i}\n",
    ] {
        assert!(imports_agents(&with_import(good)), "{good:?}");
    }
    for bad in [
        "",
        "\n",
        "AGENTS.md\n",
        "{i} \n",
        " {i}\n",
        "{i}\r\n",
        "{i_lower}\n",
        "read {i} first\n",
        "{i}.bak\n",
        "```\n{i}\n```\n",
        "~~~\n{i}\n~~~\n",
        "```md\n{i}\n```\n",
        // A shorter fence, or one of the other character, does not close the open one.
        "````\n```\n{i}\n````\n",
        "```\n~~~\n{i}\n```\n",
        // A fence line with text after it does not close.
        "```\n``` x\n{i}\n",
        // A fence never closed holds the rest of the file.
        "```\n{i}\n",
    ] {
        assert!(!imports_agents(&with_import(bad)), "{bad:?}");
    }
}

/// Catches a fence misread: four spaces of indent, a run of two, or backticks followed by a
/// backtick is no fence, so an import after it counts; up to three spaces of indent is one.
#[test]
fn a_fence_is_indented_at_most_three_spaces() {
    assert!(imports_agents(&with_import("    ```\n{i}\n")));
    assert!(!imports_agents(&with_import("   ```\n{i}\n")));
    assert!(imports_agents(&with_import("``\n{i}\n")));
    // A run of backticks with a backtick after it is inline code; a tilde fence's info string
    // may hold one.
    assert!(imports_agents(&with_import("```a`b\n{i}\n")));
    assert!(!imports_agents(&with_import("~~~a`b\n{i}\n")));
}
