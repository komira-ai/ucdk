// SPDX-License-Identifier: Apache-2.0
//! The scenario of the publisher kit about the names a release declared: a publisher that has
//! been told them stages and commits no artifact of another name.

use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::Presence;
use ucdk_release_proto::v1::Coordinate;

use super::publisher_rig::{ChannelShape, PublisherRig, Set, built, fetched, looked, publish};
use super::{must_err, same};
use crate::{CommitUnit, Publisher, SeamError};

/// A publisher told the names a release declared refuses `stage` and `commit` of an artifact of
/// another name with `SeamError::Undeclared`, which names that artifact and promises that nothing
/// happened; a set that holds one undeclared member is refused whole, and what was refused is
/// neither staged nor readable afterwards. Names compare ASCII case-insensitively, an empty
/// declaration admits nothing, and an artifact of a declared name is published as before.
pub(super) fn a_publisher_refuses_to_stage_or_commit_an_artifact_the_release_did_not_declare<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let (declared, other) = (Set::of(rig, &["a"]), Set::of(rig, &["z"]));
    let name = declared.coords[0].name.clone();
    p.declare(std::slice::from_ref(&name));
    refuses(rig, shape, &mut p, &other);
    let mixed = Set::at(
        rig,
        vec![declared.coords[0].clone(), other.coords[0].clone()],
    );
    refuses(rig, shape, &mut p, &mixed);
    publish(rig, &mut p, &declared);
    same(
        fetched(rig, &mut p, &declared.coords[0]),
        Some(declared.bytes(0).to_vec()),
        "the bytes of an artifact of a declared name",
    );

    let mut shouted = built(rig, shape);
    shouted.declare(&[name.to_ascii_uppercase()]);
    publish(rig, &mut shouted, &declared);

    let mut none = built(rig, shape);
    none.declare(&[]);
    refuses(rig, shape, &mut none, &declared);
}

/// Stage (on a channel that stages) and commit of `set` are each refused, naming the last member
/// of `set`, which is the one whose name is not declared. Nothing of `set` is staged or readable
/// afterwards.
fn refuses<R: PublisherRig>(rig: &R, shape: &ChannelShape, p: &mut R::Publisher, set: &Set) {
    let undeclared = set.coords[set.coords.len() - 1].name.clone();
    let want = SeamError::Undeclared { name: undeclared };
    let blobs = set.blobs();
    let items = set.items(&blobs);
    if p.stages() {
        let grant = rig.grant(p, GrantPhase::Stage, &set.coords);
        let error = must_err(p.stage(&grant, &items), "a stage of an undeclared artifact");
        same(
            error.promises_no_effect(),
            true,
            "a refused stage promises no effect",
        );
        same(error, want.clone(), "the refusal of a stage");
    }
    // A channel that commits each coordinate on its own is asked only for the undeclared one.
    let call = if p.commit_unit() == CommitUnit::Set {
        &items[..]
    } else {
        &items[items.len() - 1..]
    };
    let coords: Vec<Coordinate> = call.iter().map(|i| i.coordinate.clone()).collect();
    let grant = rig.grant(p, GrantPhase::Commit, &coords);
    let error = must_err(p.commit(&grant, call), "a commit of an undeclared artifact");
    same(
        error.promises_no_effect(),
        true,
        "a refused commit promises no effect",
    );
    same(error, want, "the refusal of a commit");
    for k in &set.coords {
        same(
            looked(rig, shape, p, k),
            Presence::Absent,
            "an artifact of a refused call",
        );
        same(fetched(rig, p, k), None, "the bytes of a refused call");
    }
}
