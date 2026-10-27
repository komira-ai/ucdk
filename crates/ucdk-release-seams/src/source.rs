// SPDX-License-Identifier: Apache-2.0
//! Version control, behind one seam.

use ucdk_release_proto::v1::Tri;

use crate::SeamError;

/// What the first-parent history gives a revision: the build number and the commit only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStamp {
    pub build_number: u64,
    pub source_commit: String,
}

pub trait Source {
    /// The newest first-parent commit at or below `revision` that touches anything other than
    /// the machine's `stamp_ignore` paths, and the count up to it. A shallow history, a head
    /// that is not the revision and a dirty tree are refusals.
    fn stamp(&self, revision: &str) -> Result<SourceStamp, SeamError>;

    /// Whether `a` is an ancestor of `b`; `Unknown` is never taken for either answer.
    fn is_ancestor(&self, a: &str, b: &str) -> Result<Tri, SeamError>;
}
