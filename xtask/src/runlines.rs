// SPDX-License-Identifier: Apache-2.0
//! Every `run:` line of a workflow is exactly `cargo xtask <verb>`: no expressions, no shell,
//! no second command. All logic lives in xtask, where it is tested.

use crate::Finding;

/// Whether `value` (already trimmed, so the verb is never empty) is `cargo xtask <verb>` with a
/// lower-case verb and nothing else.
fn is_xtask_line(value: &str) -> bool {
    match value.strip_prefix("cargo xtask ") {
        Some(verb) => verb.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
        None => false,
    }
}

/// The text after a `run` key, in any layout YAML allows on one line: after a dash and any
/// number of spaces, in a flow mapping (`{run: x}`), quoted (`"run": x`), or with a space before
/// the colon. A key is recognised wherever it follows the start of the line, white space, `-`,
/// `{`, `,` or `[`.
fn run_value(line: &str) -> Option<&str> {
    line.char_indices()
        .filter(|(i, _)| {
            let before = line[..*i].chars().next_back();
            before.is_none_or(|c| c.is_whitespace() || "-{,[".contains(c))
        })
        .find_map(|(i, _)| {
            let key = line[i..].trim_start_matches(['"', '\'']);
            let rest = key.strip_prefix("run")?;
            let rest = rest.trim_start_matches(['"', '\'']).trim_start();
            rest.strip_prefix(':')
        })
}

/// Whether a file is a workflow, or a composite action (whose steps also have `run:` lines).
pub fn is_workflow(path: &str) -> bool {
    let yaml = path.ends_with(".yml") || path.ends_with(".yaml");
    yaml && (path.starts_with(".github/workflows/") || path.starts_with(".github/actions/"))
}

/// `files` are (path, contents) pairs; the `.yml` and `.yaml` files under `.github/workflows/`
/// and `.github/actions/` are read.
pub fn check_run_lines(files: &[(String, String)]) -> Vec<Finding> {
    let mut out = Vec::new();
    for (path, text) in files.iter().filter(|(path, _)| is_workflow(path)) {
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with('#') {
                continue;
            }
            if let Some(value) = run_value(line)
                && !is_xtask_line(value.trim())
            {
                out.push(Finding::new(
                    "run-line",
                    format!("{path}:{}", n + 1),
                    format!("`run:{value}` is not exactly `cargo xtask <verb>`"),
                ));
            }
        }
    }
    out
}
