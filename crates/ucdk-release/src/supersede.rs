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

/// Decides from the builds held for every member of the set, one entry per held coordinate or
/// per member that holds none. The run is judged against the newest held build any member has,
/// so one member with a newer build decides for the set.
///
/// Exceptional builds are not counted, and neither is a coordinate that is only staged, which
/// is not published, nor one that is burned: nothing was built there. Yanked builds are
/// counted: a yank withdraws a coordinate, it does not un-build it. A held build is newer than
/// ours when its build number is greater. With an equal build number and another commit it is
/// newer unless it is an ancestor of ours (a commit that changes nothing a build reads keeps its
/// parent's number), so the source is asked
/// `ancestry(theirs, ours)` first and `ancestry(ours, theirs)` after it; `ancestry(a, b)` says
/// whether commit `a` is an ancestor of commit `b`. For every newer build that is not our
/// ancestor the run is `Superseded` when ours is an ancestor of all of them, `Refused` when any
/// of them is unrelated to ours (a refusal outranks a cannot-tell), and `Indeterminate` when
/// the source cannot tell, or a held build has no stamp to compare. An answer that is neither
/// yes nor no is a cannot-tell.
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
            Presence::Absent | Presence::Staged(_) | Presence::Burned => None,
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
                let back = ancestry(&theirs.source_commit, &ours.source_commit);
                if back != Tri::Yes {
                    let forward = ancestry(&ours.source_commit, &theirs.source_commit);
                    // Theirs is not known to be our ancestor. A forward no says the two are
                    // unrelated only when the backward answer was a no as well: when the source
                    // could not say whether theirs is our ancestor, the build may be one that
                    // does not count, so the answer is a cannot-tell.
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
