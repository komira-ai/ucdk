// SPDX-License-Identifier: Apache-2.0
//! Reading the repository's files for the text checks.

use std::fs;
use std::path::Path;

use crate::io_err;

/// Every file under `root` as (path relative to `root` with `/` separators, text), sorted by
/// path. Only the top-level `target` (build output, ignored by git) and `.git` are skipped; a
/// directory of either name deeper in the tree is read. A file that is not valid UTF-8 is read lossily (each bad
/// byte becomes U+FFFD), so the text around it is still scanned.
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
            if !(prefix.is_empty() && (name == "target" || name == ".git")) {
                walk(&entry.path(), &format!("{rel}/"), out)?;
            }
        } else {
            let bytes = fs::read(entry.path()).map_err(io_err)?;
            out.push((rel, String::from_utf8_lossy(&bytes).into_owned()));
        }
    }
    Ok(())
}
