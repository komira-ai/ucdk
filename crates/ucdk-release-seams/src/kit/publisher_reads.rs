// SPDX-License-Identifier: Apache-2.0
//! What a channel refuses to read for: a grant that is for another repository, has expired or
//! names no coordinate that is asked for, of any phase the channel has, and, on a private channel,
//! a stranger. Each is asked every read a channel answers (`observe` told the set and told none,
//! `fetch`, `newest` and `list`) about everything a channel can hold or lack: a coordinate the
//! caller committed, one it staged, one another writer staged, one another writer committed, one
//! nobody holds (with its name and its name as a prefix), and a channel that holds nothing. The
//! answer is the refusal, and not an empty answer for what is not there: a channel that refuses
//! only when something is held lets whoever asks find out which packages exist. A refused read is
//! never sent.

use ucdk_release::codec::GrantFault;
use ucdk_release_proto::v1::Coordinate;

use super::publisher_rig::{
    A_DAY_MS, Arrangement, ChannelShape, PublisherRig, Set, built, held_by_another, phases_of,
    publish, stage,
};
use super::same;
use crate::{Grant, ListScope, Publisher, SeamError};

/// The sets whose coordinates the reads ask about.
struct World {
    /// Committed by the caller.
    committed: Set,
    /// Staged by the caller, on a channel that stages.
    staged: Option<Set>,
    /// Staged by another writer, on a channel that stages.
    staged_by_another: Option<Set>,
    /// Committed by another writer.
    committed_by_another: Set,
    /// Held by nobody: its coordinate, its name and its name as a prefix are asked about too.
    nobody: Set,
}

impl World {
    fn of<R: PublisherRig>(rig: &R, shape: &ChannelShape) -> Self {
        Self {
            committed: Set::of(rig, &["a"]),
            staged: shape.stages.then(|| Set::of(rig, &["s"])),
            staged_by_another: shape.stages.then(|| Set::of(rig, &["o"])),
            committed_by_another: Set::of(rig, &["t"]),
            nobody: Set::of(rig, &["u"]),
        }
    }

    /// Makes the channel hold what the sets say, all but `nobody`'s.
    fn fill<R: PublisherRig>(&self, rig: &R, p: &mut R::Publisher) {
        publish(rig, p, &self.committed);
        if let Some(staged) = &self.staged {
            stage(rig, p, staged);
        }
        if let Some(theirs) = &self.staged_by_another {
            let k = &theirs.coords[0];
            rig.arrange(
                p,
                &Arrangement::StagedByAnother {
                    coordinate: k.clone(),
                    bytes: rig.payload(k, 1).bytes,
                },
            );
        }
        let k = &self.committed_by_another.coords[0];
        rig.arrange(p, &held_by_another(rig, k));
    }

    /// Each set with what it is to the channel.
    fn sets(&self) -> Vec<(&'static str, &Set)> {
        let mut all = vec![("a coordinate the caller committed", &self.committed)];
        if let Some(staged) = &self.staged {
            all.push(("a coordinate the caller staged", staged));
        }
        if let Some(theirs) = &self.staged_by_another {
            all.push(("a coordinate another writer staged", theirs));
        }
        all.push((
            "a coordinate another writer committed",
            &self.committed_by_another,
        ));
        all.push(("a coordinate nobody holds", &self.nobody));
        all
    }

    /// Every coordinate of every set.
    fn coordinates(&self) -> Vec<Coordinate> {
        self.sets()
            .into_iter()
            .flat_map(|(_, set)| set.coords.clone())
            .collect()
    }
}

/// A way to read a channel.
#[derive(Debug, Clone, Copy)]
enum Ask {
    /// An `observe` told no set.
    Observe,
    /// An `observe` told the declared set.
    ObserveTold,
    Fetch,
    /// A `newest` of the name of the coordinate.
    Newest,
    /// A `list` of the channel under the name of the coordinate as a prefix.
    List,
    /// A `list` of the whole channel.
    ListAll,
}

impl Ask {
    const ALL: [Self; 6] = [
        Self::Observe,
        Self::ObserveTold,
        Self::Fetch,
        Self::Newest,
        Self::List,
        Self::ListAll,
    ];

    /// Whether the read names a coordinate, which a grant may leave out.
    fn names_a_coordinate(self) -> bool {
        matches!(self, Self::Observe | Self::ObserveTold | Self::Fetch)
    }

    /// The read of the first coordinate of `set`.
    fn of<P: Publisher>(self, p: &mut P, g: &Grant, set: &Set) -> Result<(), SeamError> {
        let k = &set.coords[0];
        let blobs = set.blobs();
        let items = set.items(&blobs);
        let scope = |prefix: &str| ListScope {
            channel: k.channel.clone(),
            prefix: prefix.to_owned(),
        };
        match self {
            Self::Observe => p.observe(g, k, &[]).map(|_| ()),
            Self::ObserveTold => p.observe(g, k, &items).map(|_| ()),
            Self::Fetch => p.fetch(g, k).map(|_| ()),
            Self::Newest => p.newest(g, &k.channel, &k.name).map(|_| ()),
            Self::List => p.list(g, &scope(&k.name)).map(|_| ()),
            Self::ListAll => p.list(g, &scope("")).map(|_| ()),
        }
    }
}

/// What is presented to read with, and how a channel refuses it.
struct Presented {
    what: String,
    grant: Grant,
    fault: SeamError,
    /// The grant names no coordinate that is asked about: `newest` and `list` name none for a
    /// grant to leave out, and are not asked with it.
    uncovered: bool,
}

/// Holds that every read of everything `world` names, made with what is `presented`, is refused
/// with its fault and is not sent.
fn refused<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
    p: &mut R::Publisher,
    world: &World,
    presented: &Presented,
) {
    for (held, set) in world.sets() {
        for ask in Ask::ALL {
            if presented.uncovered && !ask.names_a_coordinate() {
                continue;
            }
            let what = format!("{ask:?} of {held}, with {} ({shape:?})", presented.what);
            let before = rig.sent(p);
            same(
                ask.of(p, &presented.grant, set),
                Err(presented.fault.clone()),
                &format!("the answer to {what}"),
            );
            // A grant is bad in a way the publisher sees, and it sends nothing; a stranger
            // presents no grant, and the registry is left to refuse it.
            if !presented.grant.is_none() {
                same(
                    rig.sent(p),
                    before,
                    &format!("the requests sent for {what}"),
                );
            }
        }
    }
}

/// A grant of each phase the channel has that is for another repository, one that names no
/// coordinate asked about and one that has expired is refused by every read of `world`: the answer
/// is `Denied` (`Expired` for the last) whether the channel holds the coordinate or not.
fn refuses_a_bad_grant_of_each_phase<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
    p: &mut R::Publisher,
    world: &World,
) {
    let all = world.coordinates();
    let elsewhere = Set::of(rig, &["zz"]);
    let denied = SeamError::GrantRefused(GrantFault::Denied);
    let (mut now, mut later) = (Vec::new(), Vec::new());
    for phase in phases_of(shape) {
        now.push(Presented {
            what: format!("a {phase:?} grant for another repository"),
            grant: rig.foreign_grant(p, phase, &all),
            fault: denied.clone(),
            uncovered: false,
        });
        now.push(Presented {
            what: format!("a {phase:?} grant that names no coordinate asked about"),
            grant: rig.grant(p, phase, &elsewhere.coords),
            fault: denied.clone(),
            uncovered: true,
        });
        // Good now: the grants expire when the clock moves, before they are presented.
        later.push(Presented {
            what: format!("an expired {phase:?} grant"),
            grant: rig.grant(p, phase, &all),
            fault: SeamError::GrantRefused(GrantFault::Expired),
            uncovered: false,
        });
    }
    for presented in &now {
        refused(rig, shape, p, world, presented);
    }
    rig.advance(A_DAY_MS);
    for presented in &later {
        refused(rig, shape, p, world, presented);
    }
}

/// A read with a bad grant of any phase the channel has is refused, on a channel of any shape, public
/// or private: a read that needs no grant takes no bad one, and a bad stage or commit grant is no
/// better than a bad read grant. Asked of a channel that holds each kind of coordinate and of one
/// that holds nothing.
pub(super) fn a_bad_grant_reads_nothing<R: PublisherRig>(rig: &R, shape: &ChannelShape) {
    let world = World::of(rig, shape);
    let mut filled = built(rig, shape);
    world.fill(rig, &mut filled);
    refuses_a_bad_grant_of_each_phase(rig, shape, &mut filled, &world);
    let mut empty = built(rig, shape);
    refuses_a_bad_grant_of_each_phase(rig, shape, &mut empty, &world);
}

/// Every read of a private channel needs a grant, a stranger's included: a stranger is `Denied`
/// however the read is asked, an observe told the declared set included, and whatever it asks
/// about, a coordinate held, staged or committed by another writer, or one nobody holds, and of a
/// channel that holds nothing, so that no answer tells it what exists.
pub(super) fn a_stranger_reads_nothing_of_a_private_channel<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
) {
    let world = World::of(rig, shape);
    let stranger = Presented {
        what: "no grant".to_owned(),
        grant: Grant::none(),
        fault: SeamError::GrantRefused(GrantFault::Denied),
        uncovered: false,
    };
    let mut filled = built(rig, shape);
    world.fill(rig, &mut filled);
    refused(rig, shape, &mut filled, &world, &stranger);
    let mut empty = built(rig, shape);
    refused(rig, shape, &mut empty, &world, &stranger);
}
