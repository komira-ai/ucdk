// SPDX-License-Identifier: Apache-2.0
//! Version control answers, scripted: a stamp per revision, a tri-state per pair for ancestry.

use std::collections::BTreeMap;

use ucdk_release_proto::v1::{Reason, Tri};
use ucdk_release_seams::{SeamError, Source, SourceStamp};

/// What is wrong with the checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFault {
    /// The history is shallow.
    Shallow,
    /// The tree has uncommitted changes.
    Dirty,
    /// The head is not the revision.
    HeadMismatch,
    /// The answer cannot be had: a stamp is `Unknown` and every ancestry is `UNKNOWN`.
    CannotTell,
}

#[derive(Default)]
pub struct FakeSource {
    stamps: BTreeMap<String, SourceStamp>,
    ancestry: BTreeMap<(String, String), Tri>,
    fault: Option<SourceFault>,
}

impl FakeSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// `revision` stamps as the build number and commit given.
    pub fn stamped(&mut self, revision: &str, build_number: u64, source_commit: &str) {
        self.stamps.insert(
            revision.to_owned(),
            SourceStamp {
                build_number,
                source_commit: source_commit.to_owned(),
            },
        );
    }

    /// `a` is, or is not, or may be an ancestor of `b`.
    pub fn ancestor(&mut self, a: &str, b: &str, answer: Tri) {
        self.ancestry.insert((a.to_owned(), b.to_owned()), answer);
    }

    pub fn fault(&mut self, fault: SourceFault) {
        self.fault = Some(fault);
    }
}

impl Source for FakeSource {
    fn stamp(&self, revision: &str) -> Result<SourceStamp, SeamError> {
        match self.fault {
            Some(SourceFault::Shallow | SourceFault::Dirty | SourceFault::HeadMismatch) => {
                Err(SeamError::Refused(Reason::Source))
            }
            Some(SourceFault::CannotTell) => {
                Err(SeamError::Unknown("the history cannot be read".to_owned()))
            }
            None => self
                .stamps
                .get(revision)
                .cloned()
                .ok_or(SeamError::Refused(Reason::Source)),
        }
    }

    fn is_ancestor(&self, a: &str, b: &str) -> Result<Tri, SeamError> {
        let known = self.ancestry.get(&(a.to_owned(), b.to_owned())).copied();
        Ok(match self.fault {
            Some(SourceFault::CannotTell) => Tri::Unknown,
            _ => known.unwrap_or(Tri::Unknown),
        })
    }
}
