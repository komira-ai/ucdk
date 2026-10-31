// SPDX-License-Identifier: Apache-2.0
//! The id of a group: a function of the actor and the declared set, so a client computes it, reads
//! the group with `GET /groups/{id}`, and addresses it for an abort, with no lookup by set.
//!
//! The id is `grp_` and the first 32 hex digits of the SHA-256 of the compact JSON of
//! `[actor, set]`, where the set is sorted by package name and each member is
//! `[name, version, [[subdir, filename, sha256, size], ...]]` with its files sorted by subdirectory
//! and file name. Member order, file order and the `on_overlap` option are not part of it. The
//! derivation is the one mopm documents and pins with test vectors; the tests of this crate repeat
//! those vectors.

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::mapping::Member;

/// The id of the group that `actor` declares by `members`.
pub fn group_id(actor: &str, members: &[Member]) -> String {
    let mut sorted: Vec<&Member> = members.iter().collect();
    sorted.sort_by_key(|m| m.name.clone());
    let set: Vec<Value> = sorted
        .into_iter()
        .map(|m| {
            let mut files: Vec<_> = m.files.iter().collect();
            files.sort_by_key(|f| (f.subdir.as_str(), f.file_name.clone()));
            let files: Vec<Value> = files
                .into_iter()
                .map(|f| json!([f.subdir.as_str(), f.file_name, f.sha256, f.size]))
                .collect();
            json!([m.name, m.version, files])
        })
        .collect();
    let text = json!([actor, set]).to_string();
    let hex = format!("{:x}", Sha256::digest(text.as_bytes()));
    format!("grp_{}", &hex[..32])
}

/// Whether `text` is a group id: `grp_` and 32 lower-case hex digits.
pub fn valid_group_id(text: &str) -> bool {
    text.strip_prefix("grp_").is_some_and(|hex| {
        hex.len() == 32
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}
