// SPDX-License-Identifier: Apache-2.0
//! Parts of the publisher scenarios about grants that hold what a grant says to every field of a
//! coordinate and to every shape of channel a rig declares: a registry that is strict about a bad
//! grant on a public channel that stages is held to the same on a private one, a grouped one and
//! one that does not stage.

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release::publish::Presence;
use ucdk_release_proto::v1::Coordinate;

use super::publisher_rig::{
    A_DAY_MS, Placement, PublisherRig, Set, built, commit, fetched, first_phase, observed,
    present_held, publish, stage, staged_held, staging_publisher,
};
use super::{must, must_err, same};
use crate::{Publisher, SeamError};

/// Whether `error` is the refusal of a grant that does not allow the call.
pub(super) fn denied(error: &SeamError) -> bool {
    *error == SeamError::GrantRefused(GrantFault::Denied)
}

/// What a read grant sees of `k`: on a private channel a stranger sees nothing.
pub(super) fn seen<R: PublisherRig>(rig: &R, p: &mut R::Publisher, k: &Coordinate) -> Presence {
    let grant = rig.grant(p, GrantPhase::Read, std::slice::from_ref(k));
    must(p.observe(&grant, k, &[]), "an observe with a read grant")
}

/// Holds that `call` is refused with `fault` and that the publisher sent nothing to its registry
/// for it: a grant the publisher can see is bad is refused before anything is sent, so that its
/// secret is never presented to a registry it was not issued for, and the count of
/// `PublisherRig::sent` does not move.
pub(super) fn refused_unsent<R: PublisherRig, T>(
    rig: &R,
    p: &mut R::Publisher,
    fault: &SeamError,
    call: impl FnOnce(&mut R::Publisher) -> Result<T, SeamError>,
    what: &str,
) {
    let before = rig.sent(p);
    let refusal = must_err(call(p), what);
    same(refusal, fault.clone(), &format!("the answer to {what}"));
    same(
        rig.sent(p),
        before,
        &format!("the requests sent for {what}"),
    );
}

/// Holds that `call` is `Denied` and sent nothing.
pub(super) fn denied_unsent<R: PublisherRig, T>(
    rig: &R,
    p: &mut R::Publisher,
    call: impl FnOnce(&mut R::Publisher) -> Result<T, SeamError>,
    what: &str,
) {
    refused_unsent(
        rig,
        p,
        &SeamError::GrantRefused(GrantFault::Denied),
        call,
        what,
    );
}

/// A grant names the coordinates it covers by all five fields: one that names a coordinate does
/// not stage, commit, abort, yank, observe or fetch the one of the same name that differs in its
/// channel, its version, its platform or its type, and what the refused call tried to change is
/// as it was, and nothing is sent. Held on every shape the rig declares; a channel that does not
/// stage is never asked to stage or abort. A sibling the channel takes only with its base is
/// staged and committed with it, in one call, as it is published.
pub(super) fn a_grant_names_every_field_of_the_coordinates_it_covers<R: PublisherRig>(rig: &R) {
    let base = rig.coordinate("a");
    for sibling in rig.siblings(&base) {
        let (field, asked) = (sibling.field, sibling.coordinate);
        for shape in rig.shapes() {
            let mut p = built(rig, &shape);
            let sought = match sibling.placement {
                Placement::SameSet => Set::at(rig, vec![base.clone(), asked.clone()]),
                Placement::OwnSet => Set::at(rig, vec![asked.clone()]),
            };
            // The coordinate asked for is the last of the set in either case.
            let at = sought.coords.len() - 1;
            let blobs = sought.blobs();
            let items = sought.items(&blobs);
            let near_stage = rig.grant(&p, GrantPhase::Stage, std::slice::from_ref(&base));
            let near_commit = rig.grant(&p, GrantPhase::Commit, std::slice::from_ref(&base));
            if p.stages() {
                denied_unsent(
                    rig,
                    &mut p,
                    |p| p.stage(&near_stage, &items),
                    &format!("a stage of a coordinate that differs in its {field} ({shape:?})"),
                );
            }
            let phase = first_phase(&p);
            same(
                observed(rig, &mut p, phase, &sought, at),
                Presence::Absent,
                &format!("a coordinate that differs in its {field} after the refused stage"),
            );
            stage(rig, &mut p, &sought);
            denied_unsent(
                rig,
                &mut p,
                |p| p.commit(&near_commit, &items),
                &format!("a commit of a coordinate that differs in its {field} ({shape:?})"),
            );
            same(
                fetched(rig, &mut p, &asked),
                None,
                &format!("the bytes of a coordinate that differs in its {field} after the commit"),
            );
            if p.stages() {
                denied_unsent(
                    rig,
                    &mut p,
                    |p| p.abort(&near_stage, &sought.coords),
                    &format!("an abort of a coordinate that differs in its {field} ({shape:?})"),
                );
                // A commit of what is still staged, next, finds out that it stayed.
                staged_held(observed(rig, &mut p, GrantPhase::Stage, &sought, at));
            }
            commit(rig, &mut p, &sought);
            denied_unsent(
                rig,
                &mut p,
                |p| p.yank(&near_commit, &asked, "a reason"),
                &format!("a yank of a coordinate that differs in its {field} ({shape:?})"),
            );
            present_held(seen(rig, &mut p, &asked));
            if shape.private {
                let read = rig.grant(&p, GrantPhase::Read, std::slice::from_ref(&base));
                denied_unsent(
                    rig,
                    &mut p,
                    |p| p.observe(&read, &asked, &[]),
                    &format!("an observe of a coordinate that differs in its {field}"),
                );
                denied_unsent(
                    rig,
                    &mut p,
                    |p| p.fetch(&read, &asked),
                    &format!("a fetch of a coordinate that differs in its {field}"),
                );
            }
        }
    }
}

/// An abort is a call of the stage phase: a commit grant that names the coordinate does not
/// give its staged state back, and nothing is sent.
pub(super) fn an_abort_needs_the_stage_grant_that_names_it<R: PublisherRig>(rig: &R) {
    let mut p = staging_publisher(rig);
    let set = Set::of(rig, &["a"]);
    stage(rig, &mut p, &set);
    let commit_grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    denied_unsent(
        rig,
        &mut p,
        |p| p.abort(&commit_grant, &set.coords),
        "an abort with a commit grant",
    );
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
}

/// How a grant is bad.
#[derive(Debug, Clone, Copy)]
pub(super) enum Spoiled {
    /// It is for another repository.
    Foreign,
    /// It has expired.
    Expired,
    /// It is of the right phase and names no coordinate of the call.
    Uncovered,
    /// It is the grant of the other write phase.
    OtherPhase,
    /// It is a read grant, which is no grant of either write phase, and names every coordinate.
    Read,
}

/// A stage, a commit, an abort and a yank (the first and the third are not asked of a channel
/// that does not stage) with a bad grant are each refused on every shape of channel the rig
/// declares, nothing is sent for any of them, and what each tried to change is as it was.
pub(super) fn every_shape_refuses_a_bad_write_grant<R: PublisherRig>(rig: &R, spoiled: Spoiled) {
    for shape in rig.shapes() {
        let mut p = built(rig, &shape);
        let (held, to_commit, to_abort, fresh) = (
            Set::of(rig, &["h"]),
            Set::of(rig, &["c"]),
            Set::of(rig, &["x"]),
            Set::of(rig, &["n"]),
        );
        let all = Set::of(rig, &["h", "c", "x", "n"]);
        let elsewhere = Set::of(rig, &["zz"]);
        publish(rig, &mut p, &held);
        stage(rig, &mut p, &to_commit);
        stage(rig, &mut p, &to_abort);
        let (stage_grant, commit_grant, fault) = match spoiled {
            Spoiled::Foreign => (
                rig.foreign_grant(&p, GrantPhase::Stage, &all.coords),
                rig.foreign_grant(&p, GrantPhase::Commit, &all.coords),
                GrantFault::Denied,
            ),
            Spoiled::Expired => {
                let grants = (
                    rig.grant(&p, GrantPhase::Stage, &all.coords),
                    rig.grant(&p, GrantPhase::Commit, &all.coords),
                );
                rig.advance(A_DAY_MS);
                (grants.0, grants.1, GrantFault::Expired)
            }
            Spoiled::Uncovered => (
                rig.grant(&p, GrantPhase::Stage, &elsewhere.coords),
                rig.grant(&p, GrantPhase::Commit, &elsewhere.coords),
                GrantFault::Denied,
            ),
            Spoiled::OtherPhase => (
                rig.grant(&p, GrantPhase::Commit, &all.coords),
                rig.grant(&p, GrantPhase::Stage, &all.coords),
                GrantFault::Denied,
            ),
            Spoiled::Read => (
                rig.grant(&p, GrantPhase::Read, &all.coords),
                rig.grant(&p, GrantPhase::Read, &all.coords),
                GrantFault::Denied,
            ),
        };
        let fault = SeamError::GrantRefused(fault);
        if p.stages() {
            let blobs = fresh.blobs();
            let items = fresh.items(&blobs);
            refused_unsent(
                rig,
                &mut p,
                &fault,
                |p| p.stage(&stage_grant, &items),
                &format!("a stage with a bad grant ({shape:?}, {spoiled:?})"),
            );
            refused_unsent(
                rig,
                &mut p,
                &fault,
                |p| p.abort(&stage_grant, &to_abort.coords),
                &format!("an abort with a bad grant ({shape:?}, {spoiled:?})"),
            );
        }
        let blobs = to_commit.blobs();
        let items = to_commit.items(&blobs);
        refused_unsent(
            rig,
            &mut p,
            &fault,
            |p| p.commit(&commit_grant, &items),
            &format!("a commit with a bad grant ({shape:?}, {spoiled:?})"),
        );
        same(
            fetched(rig, &mut p, &to_commit.coords[0]),
            None,
            &format!("the bytes after a commit with a bad grant ({shape:?}, {spoiled:?})"),
        );
        refused_unsent(
            rig,
            &mut p,
            &fault,
            |p| p.yank(&commit_grant, &held.coords[0], "a reason"),
            &format!("a yank with a bad grant ({shape:?}, {spoiled:?})"),
        );
        present_held(seen(rig, &mut p, &held.coords[0]));
        // What the refused calls tried to change is as it was, which a good call finds out: what
        // was refused as a stage is staged now, what was staged and refused as an abort or a
        // commit is still staged to be committed (a group-addressed channel shows staged state
        // only to an observe that is told the set, and this scenario does not depend on that).
        if p.stages() {
            stage(rig, &mut p, &fresh);
            commit(rig, &mut p, &to_abort);
        }
        commit(rig, &mut p, &to_commit);
    }
}
