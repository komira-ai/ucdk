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

/// `files` are (path, contents) pairs; only `.github/workflows/*.yml` and `*.yaml` are read.
pub fn check_run_lines(files: &[(String, String)]) -> Vec<Finding> {
    let mut out = Vec::new();
    let workflows = files.iter().filter(|(path, _)| {
        path.starts_with(".github/workflows/")
            && (path.ends_with(".yml") || path.ends_with(".yaml"))
    });
    for (path, text) in workflows {
        for (n, line) in text.lines().enumerate() {
            let line = line.trim_start();
            let line = line.strip_prefix("- ").unwrap_or(line);
            if let Some(value) = line.strip_prefix("run:")
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
