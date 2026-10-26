// SPDX-License-Identifier: Apache-2.0
//! The decision to stand down for a newer build, on a channel that never takes a build older
//! than one it holds.

use ucdk_release_proto::v1::{Reason, Stamped, Tri};

use crate::publish::Presence;

/// What a run does about the builds a channel already holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupersedeDecision {
    /// No newer build is held: go on.
    Proceed,
    /// Our revision is an ancestor of every newer build: nothing is run.
    Superseded,
    /// A newer build is held and our revision is not its ancestor.
    Refused,
    /// A held build cannot be compared, or the source cannot tell: never `Superseded`.
    Indeterminate,
}

impl SupersedeDecision {
    /// The reason a verdict carries, when the decision stops the run.
    pub fn reason(self) -> Option<Reason> {
        match self {
            SupersedeDecision::Proceed => None,
            SupersedeDecision::Superseded | SupersedeDecision::Refused => {
                Some(Reason::SupersededByNewer)
            }
            SupersedeDecision::Indeterminate => Some(Reason::CannotTell),
        }
    }
}

/// Decides from the builds held for every member of the set. The run is judged against the
/// newest held build any member has, so one member with a newer build decides for the set.
///
/// Exceptional builds are not counted. Yanked builds are: a yank withdraws a coordinate, it
/// does not un-build it. A held build is newer than ours when its build number is greater, or
/// equal with another commit that is not our ancestor. `ancestry(a, b)` says whether commit
/// `a` is an ancestor of commit `b`. A newer build that is not our descendant refuses the run,
/// even when another cannot be compared; a build with no stamp, or an answer that is not yes
/// or no, is indeterminate.
pub fn decide_supersede(
    ours: &Stamped,
    held: &[Presence],
    ancestry: &dyn Fn(&str, &str) -> Tri,
) -> SupersedeDecision {
    let mut unstamped = false;
    let mut answers = Vec::new();
    for build in held
        .iter()
        .filter_map(|p| match p {
            Presence::Absent => None,
            Presence::Present(h) | Presence::Yanked(h) => Some(h),
        })
        .filter(|h| !h.exceptional)
    {
        match &build.stamp {
            None => unstamped = true,
            Some(theirs) if theirs.build_number > ours.build_number => {
                answers.push(ancestry(&ours.source_commit, &theirs.source_commit));
            }
            Some(theirs)
                if theirs.build_number == ours.build_number
                    && theirs.source_commit != ours.source_commit =>
            {
                // A commit that changes nothing a build reads keeps its parent's build
                // number, so an equal number does not make the held build newer when it is
                // our ancestor.
                let back = ancestry(&theirs.source_commit, &ours.source_commit);
                if back != Tri::Yes {
                    let forward = ancestry(&ours.source_commit, &theirs.source_commit);
                    // When the source cannot say whether the two are unrelated, a no from
                    // the forward question is not a refusal.
                    answers.push(if back == Tri::No || forward == Tri::Yes {
                        forward
                    } else {
                        Tri::Unknown
                    });
                }
            }
            Some(_) => {}
        }
    }
    if answers.contains(&Tri::No) {
        SupersedeDecision::Refused
    } else if unstamped || answers.iter().any(|a| *a != Tri::Yes) {
        SupersedeDecision::Indeterminate
    } else if answers.is_empty() {
        SupersedeDecision::Proceed
    } else {
        SupersedeDecision::Superseded
    }
}
