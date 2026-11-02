// SPDX-License-Identifier: Apache-2.0
//! `AGENTS.md` (the rules every change is written and reviewed against) and `CLAUDE.md`
//! (which imports it). The checks are functions of the texts, so each is tested on edited
//! texts as well as on the real files.

use std::fs;

use xtask::agents::{
    AGENTS, BEGIN, CLAUDE, COPY, Core, END, HASH, IMPORT, core, hash_line, hash_matches,
    imports_agents, same_core,
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
    for id in ["T1", "B8", "B9", "P1", "S1"] {
        let rule = format!("- {id} ");
        assert!(
            lines.iter().any(|l| l.starts_with(&rule)),
            "{COPY} has no rule {id}"
        );
    }
}

/// Catches a rule that is the reviewer's alone, or meant only for the project's own sessions,
/// written into this public file: no rule of the interface and design review (F, G, S5) or of
/// the hygiene layer (H) is stated in `AGENTS.md`.
#[test]
fn agents_md_states_no_reviewer_only_or_internal_rule() {
    let agents = read(AGENTS);
    let stated: Vec<&str> = agents
        .lines()
        .filter_map(|l| l.strip_prefix("- "))
        .filter_map(|l| l.split(' ').next())
        .filter(|id| {
            let mut chars = id.chars();
            matches!(chars.next(), Some('F' | 'G' | 'H'))
                && chars.next().is_some_and(|c| c.is_ascii_digit())
                || *id == "S5"
        })
        .collect();
    assert!(stated.is_empty(), "{AGENTS} states {stated:?}");
}

fn agents_with(core_text: &str) -> String {
    format!("# AGENTS.md\n\nintro\n\n{BEGIN}\n{core_text}{END}\n\n## 2. Rust\n")
}

const CORE: &str = "## 1. Shared core\n\n- T1 BLOCK. one\n- T2 SHOULD. two\n";

/// A line as the diff message quotes it.
fn q(line: &str) -> String {
    format!("{line:?}")
}

fn found(text: &str, lines: std::ops::Range<usize>) -> Result<Core<'_>, Vec<String>> {
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

/// Catches a marker that is missing, repeated, out of order, or not a line of its own.
#[test]
fn markers_that_cannot_delimit_the_core_are_refused() {
    let times =
        |m: &str, n: usize| format!("AGENTS.md: the line `{m}` is there {n} times, not once");
    assert_eq!(core(""), Err(vec![times(BEGIN, 0), times(END, 0)]));
    assert_eq!(
        core("no markers\n"),
        Err(vec![times(BEGIN, 0), times(END, 0)])
    );
    assert_eq!(core(&format!("{BEGIN}\nx\n")), Err(vec![times(END, 0)]));
    assert_eq!(core(&format!("x\n{END}\n")), Err(vec![times(BEGIN, 0)]));
    assert_eq!(
        core(&format!("{BEGIN}\n{BEGIN}\nx\n{END}\n")),
        Err(vec![times(BEGIN, 2)])
    );
    assert_eq!(
        core(&format!("{BEGIN}\nx\n{END}\n{END}\n")),
        Err(vec![times(END, 2)])
    );
    assert_eq!(
        core(&format!("{BEGIN}\n{BEGIN}\n{END}\n{END}\n")),
        Err(vec![times(BEGIN, 2), times(END, 2)])
    );
    assert_eq!(
        core(&format!("{END}\nx\n{BEGIN}\n")),
        Err(vec![
            "AGENTS.md:1: the end of the shared core comes before its begin (line 3)".to_owned()
        ])
    );
    // A marker with text around it, or a carriage return, is not a marker line.
    for bad in [
        format!(" {BEGIN}"),
        format!("{BEGIN} "),
        format!("{BEGIN}\r"),
        format!("see {BEGIN}"),
    ] {
        assert_eq!(
            core(&format!("{bad}\nx\n{END}\n")),
            Err(vec![times(BEGIN, 0)]),
            "{bad:?}"
        );
    }
}

/// Catches an edited core: a changed, added or removed line, a carriage return, or a
/// difference only in the final newline, each named by its line in AGENTS.md and in the copy.
#[test]
fn a_core_that_differs_from_the_copy_names_the_first_differing_line() {
    let at = |text| Core { text, lines: 6..10 };
    let differs = |agents_line: usize, copy_line: usize, got: &str, want: &str| {
        Err(format!(
            "AGENTS.md:{agents_line}: the shared core differs from {COPY}:{copy_line}: AGENTS.md has {got}, the copy has {want}"
        ))
    };
    assert_eq!(same_core(&at(CORE), CORE), Ok(()));
    let changed = CORE.replace("two", "2");
    assert_eq!(
        same_core(&at(&changed), CORE),
        differs(9, 4, &q("- T2 SHOULD. 2\n"), &q("- T2 SHOULD. two\n"))
    );
    let added = format!("{CORE}- T3 BLOCK. three\n");
    assert_eq!(
        same_core(&at(&added), CORE),
        differs(10, 5, &q("- T3 BLOCK. three\n"), "the end of the text")
    );
    assert_eq!(
        same_core(&at(CORE), &added),
        differs(10, 5, "the end of the text", &q("- T3 BLOCK. three\n"))
    );
    let crlf = CORE.replacen('\n', "\r\n", 1);
    assert_eq!(
        same_core(&at(&crlf), CORE),
        differs(6, 1, &q("## 1. Shared core\r\n"), &q("## 1. Shared core\n"))
    );
    assert_eq!(
        same_core(&at(CORE.trim_end_matches('\n')), CORE),
        differs(9, 4, &q("- T2 SHOULD. two"), &q("- T2 SHOULD. two\n"))
    );
    // A core emptied in AGENTS.md alone.
    assert_eq!(
        same_core(&at(""), CORE),
        differs(6, 1, "the end of the text", &q("## 1. Shared core\n"))
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
        assert_eq!(
            hash_matches(copy, &bad),
            Err(format!(
                "{HASH} is {bad:?}, not {good:?} (run `sha256sum shared_core.md` in docs/agents)"
            )),
            "{bad:?}"
        );
    }
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
