// SPDX-License-Identifier: Apache-2.0
//! The comments of the protos, found by descriptor path in the source info.

use prost_reflect::prost_types::FileDescriptorProto;

/// The leading comment of the element at `path` in `file`, with the one space after `//`
/// removed from each line; `None` when it has none (or `file` carries no source info).
pub fn leading(file: &FileDescriptorProto, path: &[i32]) -> Option<String> {
    let info = file.source_code_info.as_ref()?;
    let location = info.location.iter().find(|l| l.path == path)?;
    let comment = location.leading_comments.as_deref()?;
    let lines: Vec<&str> = comment
        .lines()
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .collect();
    Some(lines.join("\n").trim().to_owned())
}
