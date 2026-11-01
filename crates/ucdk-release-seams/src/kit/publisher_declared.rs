// SPDX-License-Identifier: Apache-2.0
//! The scenarios of the publisher kit about the names a release declared: a publisher stages and
//! commits no artifact of a name it was not told, and it was told nothing until `declare` is
//! called; a declaration that is not a list of plain, distinct names is refused whole.

use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::Presence;
use ucdk_release_proto::v1::Coordinate;

use super::publisher_rig::{
    ChannelShape, PublisherRig, Set, built_undeclared, commit, fetched, looked, observed_as_staged,
    publish, stage, staged_held,
};
use super::{must, must_err, same};
use crate::{CommitUnit, DeclareError, Publisher, SeamError};

/// A publisher told the names a release declared refuses `stage` and `commit` of an artifact of
/// another name with `SeamError::Undeclared`, which names that artifact and promises that nothing
/// happened; a set that holds one undeclared member is refused whole, and what was refused is
/// neither staged nor readable afterwards. Names compare ASCII case-insensitively, an empty
/// declaration admits nothing, and an artifact of a declared name is published as before. A
/// declaration made between a stage and its commit governs the commit: the commit of a name that
/// is no longer declared is refused and the staged state stays until it is aborted.
pub(super) fn a_publisher_refuses_to_stage_or_commit_an_artifact_the_release_did_not_declare<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built_undeclared(rig, shape);
    let (declared, other) = (Set::of(rig, &["a"]), Set::of(rig, &["z"]));
    let name = declared.coords[0].name.clone();
    must(p.declare(std::slice::from_ref(&name)), "a declaration");
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

    let mut shouted = built_undeclared(rig, shape);
    must(
        shouted.declare(&[name.to_ascii_uppercase()]),
        "a declaration in upper case",
    );
    publish(rig, &mut shouted, &declared);

    let mut none = built_undeclared(rig, shape);
    must(none.declare(&[]), "an empty declaration");
    refuses(rig, shape, &mut none, &declared);

    a_declaration_between_a_stage_and_its_commit_governs_the_commit(rig, shape);
}

/// A declaration that no longer names `a` refuses the commit of `a`, which stays staged on a
/// channel that stages until it is aborted; declared again, it commits.
fn a_declaration_between_a_stage_and_its_commit_governs_the_commit<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built_undeclared(rig, shape);
    let (set, other) = (Set::of(rig, &["a"]), Set::of(rig, &["z"]));
    let name = set.coords[0].name.clone();
    must(p.declare(std::slice::from_ref(&name)), "a declaration");
    stage(rig, &mut p, &set);
    must(
        p.declare(std::slice::from_ref(&other.coords[0].name)),
        "a second declaration",
    );
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    same(
        must_err(
            p.commit(&grant, &items),
            "a commit of a name no longer declared",
        ),
        SeamError::Undeclared { name: name.clone() },
        "the refusal of that commit",
    );
    same(
        fetched(rig, &mut p, &set.coords[0]),
        None,
        "the bytes of a refused commit",
    );
    if p.stages() {
        staged_held(observed_as_staged(
            rig,
            shape,
            &mut p,
            GrantPhase::Stage,
            &set,
            0,
        ));
        let grant = rig.grant(&p, GrantPhase::Stage, &set.coords);
        must(p.abort(&grant, &set.coords), "an abort of refused state");
        same(
            observed_as_staged(rig, shape, &mut p, GrantPhase::Stage, &set, 0),
            Presence::Absent,
            "the staged state after the abort",
        );
        must(
            p.declare(std::slice::from_ref(&name)),
            "a third declaration",
        );
        stage(rig, &mut p, &set);
    }
    must(
        p.declare(std::slice::from_ref(&name)),
        "a fourth declaration",
    );
    commit(rig, &mut p, &set);
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(set.bytes(0).to_vec()),
        "the bytes committed once the name is declared again",
    );
}

/// A publisher that was never told any names refuses every stage and commit with
/// `SeamError::Undeclared`, which names the artifact: a publisher fails closed, and holds a
/// declaration only because it was given one.
pub(super) fn a_publisher_never_told_any_names_refuses<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built_undeclared(rig, shape);
    refuses(rig, shape, &mut p, &Set::of(rig, &["a"]));
}

/// A declaration with an empty name, a name that holds whitespace or a `/`, or a name that is an
/// earlier one again ignoring ASCII case is refused with the error that names the entry, and
/// changes nothing: what was declared before is declared still, and no entry of the refused list
/// is.
pub(super) fn a_declaration_that_is_not_plain_distinct_names_is_refused_and_changes_nothing<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built_undeclared(rig, shape);
    let (declared, other) = (Set::of(rig, &["a"]), Set::of(rig, &["z"]));
    let (name, z) = (
        declared.coords[0].name.clone(),
        other.coords[0].name.clone(),
    );
    must(p.declare(std::slice::from_ref(&name)), "a declaration");
    let spaced = format!("{name} {z}");
    let slashed = format!("{name}/{z}");
    let padded = format!(" {z}");
    let shouted = name.to_ascii_uppercase();
    let refused = [
        (
            vec![z.clone(), String::new()],
            DeclareError::Empty { index: 1 },
        ),
        (
            vec![spaced.clone()],
            DeclareError::Whitespace { name: spaced },
        ),
        (
            vec![padded.clone()],
            DeclareError::Whitespace { name: padded },
        ),
        (vec![slashed.clone()], DeclareError::Slash { name: slashed }),
        (
            vec![z.clone(), name.clone(), shouted.clone()],
            DeclareError::Duplicate { name: shouted },
        ),
        (
            vec![z.clone(), z.clone()],
            DeclareError::Duplicate { name: z },
        ),
    ];
    for (names, want) in refused {
        same(
            p.declare(&names),
            Err(want),
            &format!("the declaration {names:?}"),
        );
    }
    refuses(rig, shape, &mut p, &other);
    publish(rig, &mut p, &declared);
    same(
        fetched(rig, &mut p, &declared.coords[0]),
        Some(declared.bytes(0).to_vec()),
        "the bytes of the name declared before the refused declarations",
    );
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
