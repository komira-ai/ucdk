// SPDX-License-Identifier: Apache-2.0
//! Scenarios of the publisher kit about what a rig declares and what a registry does when a
//! world goes wrong in a way only some registries can be put into: a vanished reservation, a
//! commit that becomes readable late, a coordinate the channel cannot hold, a sealed version, a
//! burned number, and a registry that echoes the digest a writer declared.

use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_proto::v1::{Coordinate, Reason, Stamped};

use super::publisher_rig::{
    Arrangement, ChannelShape, PublisherRig, Set, built, fetched, fetched_by_a_stranger,
    needs_a_plain_channel, observed, plain_shapes, present_held, publish, sha, siblings_beside,
    stage, staged_held, staging_publisher, stranger,
};
use super::{must, must_err, must_some, same};
use crate::{
    CommitAnswer, Grant, ListScope, ObserveFailure, ObserveWhen, Publisher, SeamError, StageAnswer,
};

/// How many coordinates the largest set of the kit holds, and so how many one grant must cover.
const LARGEST_SET: usize = 4;

/// How many times an observe is asked, after a commit, before the coordinate must be readable:
/// the arrangement makes it readable at the third.
const LAG: u32 = 2;

pub(super) fn needs_an_unrepresentable_coordinate<R: PublisherRig>(
    rig: &R,
) -> Option<&'static str> {
    needs_a_plain_channel(rig).or_else(|| {
        rig.unrepresentable()
            .is_empty()
            .then_some("the rig names no coordinate its channel cannot hold")
    })
}

pub(super) fn needs_a_sealing_registry<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    needs_a_plain_channel(rig).or_else(|| {
        (!rig.seals_versions()).then_some("the registry does not seal a version by its first group")
    })
}

pub(super) fn needs_a_registry_that_can_burn<R: PublisherRig>(rig: &R) -> Option<&'static str> {
    needs_a_plain_channel(rig)
        .or_else(|| (!rig.can_burn()).then_some("the registry cannot burn a number"))
}

/// A coordinate whose staged state vanished is not committed, and nobody holds it: the commit is
/// `Refused` or `Permanent` (an answer that asks to be retried, such as `RateLimited`, would be
/// retried for ever instead of staged again), an observe finds `Absent`, and a new stage of it
/// works. This is the world in which the reservation was lost; it is not the
/// race of another writer, whose commit makes the version taken and answers `Duplicate`.
pub(super) fn a_commit_of_staged_state_that_vanished_is_not_committed_and_a_restage_works<
    R: PublisherRig,
>(
    rig: &R,
) {
    let mut p = staging_publisher(rig);
    let set = Set::of(rig, &["a"]);
    stage(rig, &mut p, &set);
    rig.arrange(&mut p, &Arrangement::LoseStagedState);
    let blobs = set.blobs();
    let items = set.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    let refusal = must_err(
        p.commit(&grant, &items),
        "a commit of staged state that vanished",
    );
    same(
        matches!(refusal, SeamError::Refused(_) | SeamError::Permanent(_)),
        true,
        &format!(
            "the answer to a commit of staged state that vanished, {refusal:?}, is Refused or Permanent"
        ),
    );
    same(
        observed(rig, &mut p, GrantPhase::Stage, &set, 0),
        Presence::Absent,
        "a coordinate whose staged state vanished",
    );
    same(
        fetched(rig, &mut p, &set.coords[0]),
        None,
        "the bytes of a coordinate whose staged state vanished",
    );
    stage(rig, &mut p, &set);
    staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, 0));
    super::publisher_rig::commit(rig, &mut p, &set);
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(set.bytes(0).to_vec()),
        "the bytes committed after a new stage",
    );
}

/// A commit that is repeated while the first is becoming readable (its answer was not seen) is
/// `Committed` or `Duplicate`, whichever the registry says, and never a second write: the
/// coordinate then reads as `Absent` until it is readable, and as `Present` with the `Held` of the
/// first commit (the digest of the bytes that were sent, the stamp and the mark it declared),
/// whose bytes are the first ones. The repeat is asked three times: as it was, with another
/// stamp, and with another mark, so that a registry that writes the coordinate again with what
/// the repeat declares is caught.
pub(super) fn a_retry_of_a_commit_that_is_becoming_readable_is_committed_or_duplicate_and_never_a_second_write<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let set = Set::of(rig, &["a"]);
    let mut restamped = set.clone();
    restamped.held.stamp = Some(Stamped {
        build_number: 42,
        source_commit: "def456".to_owned(),
    });
    let mut marked = set.clone();
    marked.held.exceptional = true;
    for (what, repeat) in [
        ("as it was", &set),
        ("with another stamp", &restamped),
        ("with another mark", &marked),
    ] {
        a_retry_is_not_a_second_write(rig, shape, what, &set, repeat);
    }
}

/// One repeat of the commit of `set`, `what` it declares, on a channel of its own, so that a
/// later repeat does not put back what an earlier one replaced.
fn a_retry_is_not_a_second_write<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
    what: &str,
    set: &Set,
    repeat: &Set,
) {
    let mut p = built(rig, shape);
    stage(rig, &mut p, set);
    rig.arrange(&mut p, &Arrangement::CommitVisibleAfter(LAG));
    super::publisher_rig::commit(rig, &mut p, set);
    let blobs = repeat.blobs();
    let items = repeat.items(&blobs);
    let grant = rig.grant(&p, GrantPhase::Commit, &repeat.coords);
    let retry = must(p.commit(&grant, &items), "a retry of a commit");
    same(
        matches!(retry, CommitAnswer::Committed | CommitAnswer::Duplicate),
        true,
        &format!("the answer to a retry {what} of a commit that is becoming readable"),
    );
    let mut seen = None;
    for _ in 0..=LAG + 1 {
        match stranger(&mut p, &set.coords[0]) {
            Presence::Present(held) => {
                seen = Some(held);
                break;
            }
            other => same(
                other,
                Presence::Absent,
                "a coordinate that is becoming readable",
            ),
        }
    }
    let held = must_some(
        seen,
        "the coordinate, readable after the observes it needed",
    );
    same(
        held,
        Held {
            digest: set.digest(0),
            ..set.held.clone()
        },
        &format!("what a coordinate holds after a retry {what}: what the first commit declared"),
    );
    same(
        fetched(rig, &mut p, &set.coords[0]),
        Some(set.bytes(0).to_vec()),
        &format!("the bytes after a retry {what} of a commit"),
    );
}

/// The answer of a call that must be `Refused(Channel)`.
fn refused_as_channel<T>(result: Result<T, SeamError>, what: &str) -> SeamError {
    let error = must_err(result, what);
    same(
        error.clone(),
        SeamError::Refused(Reason::Channel),
        &format!("the answer of {what}"),
    );
    error
}

/// A coordinate the channel cannot hold is refused by every call that names it, before anything
/// is sent: an observe of it is `Refused` and not `Absent`, which the engine reads as a refusal
/// of the action and not as a coordinate to create, and the channel is as it was.
pub(super) fn a_coordinate_the_channel_cannot_hold_is_refused_before_anything_is_sent<
    R: PublisherRig,
>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    for k in rig.unrepresentable() {
        let one = std::slice::from_ref(&k);
        let set = Set::at(rig, one.to_vec());
        let blobs = set.blobs();
        let items = set.items(&blobs);
        let seen = refused_as_channel(p.observe(&Grant::none(), &k, &[]), "an observe");
        same(
            seen.observe_failure(ObserveWhen::Preflight),
            ObserveFailure::Refused(Reason::Channel),
            "what the failed observe makes of the action",
        );
        refused_as_channel(
            p.observe(&rig.grant(&p, GrantPhase::Read, one), &k, &items),
            "an observe with a grant and the set",
        );
        if p.stages() {
            let grant = rig.grant(&p, GrantPhase::Stage, one);
            refused_as_channel(p.stage(&grant, &items), "a stage");
            refused_as_channel(p.abort(&grant, one), "an abort");
        }
        let grant = rig.grant(&p, GrantPhase::Commit, one);
        refused_as_channel(p.commit(&grant, &items), "a commit");
        refused_as_channel(p.yank(&grant, &k, "a reason"), "a yank");
        let read = rig.grant(&p, GrantPhase::Read, one);
        refused_as_channel(p.fetch(&read, &k), "a fetch");
        a_set_with_such_a_coordinate_applies_none_of_it(rig, shape, &k);
    }
    // What was refused left the channel as it was: an ordinary coordinate publishes.
    let set = Set::of(rig, &["a"]);
    publish(rig, &mut p, &set);
    present_held(stranger(&mut p, &set.coords[0]));
}

/// A set that has a coordinate the channel cannot hold beside one it can is refused whole, at
/// the stage and at the commit: the coordinate beside it is not staged by the stage and not
/// readable after the commit.
fn a_set_with_such_a_coordinate_applies_none_of_it<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
    k: &Coordinate,
) {
    let mut channel = built(rig, shape);
    let p = &mut channel;
    let beside = Set::of(rig, &["b"]);
    let both = Set::at(rig, vec![beside.coords[0].clone(), k.clone()]);
    let blobs = both.blobs();
    let items = both.items(&blobs);
    if p.stages() {
        let grant = rig.grant(p, GrantPhase::Stage, &both.coords);
        refused_as_channel(p.stage(&grant, &items), "a stage of a set with it");
        same(
            observed(rig, p, GrantPhase::Stage, &beside, 0),
            Presence::Absent,
            "the coordinate beside it after a refused stage",
        );
        stage(rig, p, &beside);
    }
    let grant = rig.grant(p, GrantPhase::Commit, &both.coords);
    refused_as_channel(p.commit(&grant, &items), "a commit of a set with it");
    same(
        fetched(rig, p, &beside.coords[0]),
        None,
        "the bytes of the coordinate beside it after a refused commit",
    );
}

/// A registry that seals a version by the group that first wrote it refuses a later platform of
/// the same name and version with `Refused(Channel)`, which promises that nothing happened, and
/// reads it `Absent`. Never `Duplicate`: an observe that answers `Absent` after a `Duplicate`
/// would make the caller stage it again for ever.
pub(super) fn a_sealed_version_refuses_a_later_platform_and_observes_it_absent<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let base = rig.coordinate("a");
    let platform = must_some(
        rig.siblings(&base)
            .into_iter()
            .find(|s| s.field == "platform"),
        "a platform of the base coordinate in the siblings the rig names",
    );
    let first = Set::at(rig, vec![base.clone()]);
    publish(rig, &mut p, &first);
    let later = Set::at(rig, vec![platform.coordinate.clone()]);
    let blobs = later.blobs();
    let items = later.items(&blobs);
    let staged = if p.stages() {
        let grant = rig.grant(&p, GrantPhase::Stage, &later.coords);
        p.stage(&grant, &items)
    } else {
        Ok(StageAnswer::Staged)
    };
    let refusal = match staged {
        Err(refusal) => refusal,
        Ok(_) => {
            let grant = rig.grant(&p, GrantPhase::Commit, &later.coords);
            must_err(p.commit(&grant, &items), "a commit of a later platform")
        }
    };
    same(
        refusal,
        SeamError::Refused(Reason::Channel),
        "the answer to a later platform of a sealed version",
    );
    same(
        stranger(&mut p, &platform.coordinate),
        Presence::Absent,
        "a later platform of a sealed version",
    );
    same(
        fetched(rig, &mut p, &platform.coordinate),
        None,
        "the bytes of a later platform of a sealed version",
    );
    present_held(stranger(&mut p, &base));
}

/// A burned coordinate is neither absent nor present nor committable: an observe says `Burned`,
/// a stage and a commit over it answer `Duplicate` and never `Committed`, a fetch finds nothing
/// and `newest` and `list` do not name it. Only a registry that can burn a number runs this.
pub(super) fn a_burned_coordinate_is_neither_absent_nor_present_nor_committable<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let set = Set::of(rig, &["a"]);
    let a = &set.coords[0];
    rig.arrange(
        &mut p,
        &Arrangement::BurnedByFailedGroup {
            coordinate: a.clone(),
        },
    );
    same(stranger(&mut p, a), Presence::Burned, "a burned coordinate");
    same(
        observed(rig, &mut p, GrantPhase::Commit, &set, 0),
        Presence::Burned,
        "a burned coordinate observed with a grant and the set",
    );
    let blobs = set.blobs();
    let items = set.items(&blobs);
    if p.stages() {
        let grant = rig.grant(&p, GrantPhase::Stage, &set.coords);
        same(
            must(p.stage(&grant, &items), "a stage over a burned coordinate"),
            StageAnswer::Duplicate,
            "a stage over a burned coordinate",
        );
    }
    let grant = rig.grant(&p, GrantPhase::Commit, &set.coords);
    same(
        must(
            p.commit(&grant, &items),
            "a commit over a burned coordinate",
        ),
        CommitAnswer::Duplicate,
        "a commit over a burned coordinate",
    );
    same(
        fetched(rig, &mut p, a),
        None,
        "the bytes of a burned coordinate",
    );
    let read = rig.grant(&p, GrantPhase::Read, &set.coords);
    same(
        must(p.newest(&read, &a.channel, &a.name), "newest").is_empty(),
        true,
        "a burned coordinate named by newest",
    );
    let scope = ListScope {
        channel: a.channel.clone(),
        prefix: String::new(),
    };
    same(
        must(p.list(&read, &scope), "list").is_empty(),
        true,
        "a burned coordinate named by list",
    );
}

/// Whether the read-back of `k` on a public channel passes: the bytes are fetched as a stranger
/// (`Grant::none()`) and hashed by the reader, and what they hash to is the digest the action
/// expects. The scenarios of this part run on public channels only.
fn read_back_passes<P: Publisher>(p: &mut P, k: &Coordinate, expected: &str) -> bool {
    fetched_by_a_stranger(p, k).is_some_and(|bytes| sha(&bytes) == expected)
}

/// A registry reports, from `observe`, the digest it verified over the bytes it stored, and not
/// the digest the writer declared: the kit declares one that is not the hash of the member, and
/// requires that what `observe` reports is the hash of the bytes the kit sent. The read-back
/// (`CHANNEL_READBACK` and a smoke action fetch the bytes and hash them themselves) passes the
/// digest of the bytes and fails the one that was declared, whatever `observe` reported, so a
/// registry that echoes the declared digest is caught here and could not pass a read-back. It is
/// held for every member of a set of several, on every declared channel, and for a coordinate
/// that was yanked.
pub(super) fn a_registry_that_echoes_the_declared_digest_fails_readback<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let mut p = built(rig, shape);
    let mut set = Set::of(rig, &["a", "b"]);
    let declared = sha(b"bytes that are not the member");
    set.held.digest.clone_from(&declared);
    if p.stages() {
        stage(rig, &mut p, &set);
        for at in 0..set.coords.len() {
            let staged = staged_held(observed(rig, &mut p, GrantPhase::Stage, &set, at));
            same(
                staged.digest,
                set.digest(at),
                "the digest a staged coordinate is observed with, from the bytes that were sent",
            );
        }
    }
    super::publisher_rig::commit(rig, &mut p, &set);
    for (at, k) in set.coords.iter().enumerate() {
        let held = present_held(stranger(&mut p, k));
        same(
            held.digest,
            set.digest(at),
            "the digest a committed coordinate is observed with, from the bytes that were sent",
        );
        same(
            read_back_passes(&mut p, k, &set.digest(at)),
            true,
            "the read-back against the digest of the bytes that were sent",
        );
        same(
            read_back_passes(&mut p, k, &declared),
            false,
            "the read-back against a digest that was only declared",
        );
    }
    let a = &set.coords[0];
    let grant = rig.grant(&p, GrantPhase::Commit, std::slice::from_ref(a));
    must(p.yank(&grant, a, "a reason"), "yank");
    same(
        stranger(&mut p, a),
        Presence::Yanked(Held {
            digest: set.digest(0),
            ..set.held.clone()
        }),
        "what a yanked coordinate is observed with: the digest from the bytes that were sent",
    );
}

/// What a rig declares holds of what it builds: it declares a shape, each channel it builds says
/// what its shape says, the channel takes the largest set of the kit under one grant, and the
/// coordinates, members, siblings and unrepresentable coordinates it supplies are the kind the
/// kit relies on.
pub(super) fn the_declarations_of_a_rig_hold_of_what_it_builds<R: PublisherRig>(rig: &R) {
    let shapes = rig.shapes();
    same(
        shapes.is_empty(),
        false,
        "the rig declares the shapes of channel it builds",
    );
    for shape in &shapes {
        built(rig, shape);
    }
    for shape in plain_shapes(rig) {
        let mut p = built(rig, &shape);
        same(
            p.grant_limit() >= LARGEST_SET,
            true,
            "the grant limit of a channel against the largest set the kit publishes",
        );
        let labels: Vec<String> = (0..LARGEST_SET).map(|i| format!("g{i}")).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let set = Set::of(rig, &labels);
        publish(rig, &mut p, &set);
        for k in &set.coords {
            present_held(stranger(&mut p, k));
        }
    }
    coordinates_keep_their_labels(rig);
    members_depend_on_their_seed(rig);
    siblings_are_named(rig);
    siblings_differ_in_their_field(rig);
}

/// The rig names a sibling the scenarios can publish beside the base: the checks about siblings
/// are parts of scenarios about other things, and run over what the rig names, so a rig that
/// names none would pass them without being asked.
fn siblings_are_named<R: PublisherRig>(rig: &R) {
    same(
        siblings_beside(rig, &rig.coordinate("a")).is_empty(),
        false,
        "the siblings the rig names that the scenarios publish beside the base",
    );
}

/// A coordinate is distinct for each label, and its name starts with the label, because the
/// kit lists by a prefix that is a label.
fn coordinates_keep_their_labels<R: PublisherRig>(rig: &R) {
    let labels = ["a", "ab", "b", "u", "x"];
    let coords: Vec<Coordinate> = labels.iter().map(|l| rig.coordinate(l)).collect();
    for (label, k) in labels.iter().zip(&coords) {
        same(
            k.name.starts_with(label),
            true,
            &format!("the name the rig gives the label {label:?} starts with it"),
        );
    }
    for (at, k) in coords.iter().enumerate() {
        same(
            coords[..at].contains(k),
            false,
            "two labels give two different coordinates",
        );
    }
}

/// A seed names one member, and two seeds name two different ones.
fn members_depend_on_their_seed<R: PublisherRig>(rig: &R) {
    let k = rig.coordinate("a");
    same(
        rig.payload(&k, 0),
        rig.payload(&k, 0),
        "the member of a seed asked for twice",
    );
    same(
        rig.payload(&k, 0) == rig.payload(&k, 1),
        false,
        "the members of two seeds",
    );
    same(
        rig.payload(&k, 0).bytes.is_empty(),
        false,
        "the bytes of a member",
    );
}

/// A sibling differs from its base in the field it names and in no other.
fn siblings_differ_in_their_field<R: PublisherRig>(rig: &R) {
    let base = rig.coordinate("a");
    for sibling in rig.siblings(&base) {
        let (b, s) = (&base, &sibling.coordinate);
        let differs = [
            ("channel", b.channel != s.channel),
            ("version", b.version != s.version),
            ("platform", b.platform != s.platform),
            ("type", b.r#type != s.r#type),
        ];
        same(
            differs
                .iter()
                .filter(|(_, differs)| *differs)
                .map(|(field, _)| *field)
                .collect::<Vec<_>>(),
            vec![sibling.field],
            "the fields in which a sibling differs from its base",
        );
        same(b.name == s.name, true, "the name of a sibling");
    }
    for k in rig.unrepresentable() {
        same(
            k == base,
            false,
            "a coordinate the channel cannot hold is not the base of the scenarios",
        );
    }
}
