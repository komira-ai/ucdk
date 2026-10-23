// SPDX-License-Identifier: Apache-2.0
//! Every `.rs` and `.proto` file begins with the licence header.

use crate::Finding;

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
