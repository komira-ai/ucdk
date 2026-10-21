// SPDX-License-Identifier: Apache-2.0
//! Every `.rs` and `.proto` file begins with the licence header.

use std::fs;
use std::io;
use std::path::Path;

use crate::{Finding, to_result};

pub const HEADER: &str = "// SPDX-License-Identifier: Apache-2.0";

/// `files` are (path, contents) pairs.
pub fn check_spdx(files: &[(String, String)]) -> Vec<Finding> {
    files
        .iter()
        .filter(|(path, _)| path.ends_with(".rs") || path.ends_with(".proto"))
        .filter(|(_, text)| text.lines().next() != Some(HEADER))
        .map(|(path, _)| {
            Finding::new(
                "spdx-missing",
                path,
                format!("first line must be `{HEADER}`"),
            )
        })
        .collect()
}

/// Reads every `.rs` and `.proto` file under `root`, skipping `target` and `.git`.
pub fn collect(root: &Path) -> io::Result<Vec<(String, String)>> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) -> io::Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for e in entries {
            let path = e.path();
            let name = e.file_name();
            if e.file_type()?.is_dir() {
                if name != "target" && name != ".git" {
                    walk(&path, root, out)?;
                }
            } else if matches!(
                path.extension().and_then(|x| x.to_str()),
                Some("rs" | "proto")
            ) {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, fs::read_to_string(&path)?));
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

pub fn run(root: &Path) -> Result<(), Vec<Finding>> {
    let files = collect(root).map_err(|e| vec![Finding::new("io", "walk", e.to_string())])?;
    to_result(check_spdx(&files))
}
