// SPDX-License-Identifier: Apache-2.0
//! Reading the repository's files for the text checks.

use std::fs;
use std::path::Path;

use crate::io_err;

/// Every UTF-8 file under `root` as (path relative to `root` with `/` separators, text), sorted
/// by path. `target` and `.git` are skipped; a file that is not UTF-8 text is skipped.
pub fn read_tree(root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    walk(root, "", &mut out)?;
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, String)>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(io_err)?
        .collect::<Result<_, _>>()
        .map_err(io_err)?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = format!("{prefix}{name}");
        if entry.file_type().map_err(io_err)?.is_dir() {
            if name != "target" && name != ".git" {
                walk(&entry.path(), &format!("{rel}/"), out)?;
            }
        } else if let Ok(text) = fs::read_to_string(entry.path()) {
            out.push((rel, text));
        }
    }
    Ok(())
}
