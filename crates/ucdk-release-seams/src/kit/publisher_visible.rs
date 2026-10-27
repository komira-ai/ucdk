// SPDX-License-Identifier: Apache-2.0
//! What the grants of a channel see of the staged state of another writer, which is nothing: a
//! reader of any phase, told the declared set or not, finds nothing of it by `observe`, `fetch`,
//! `newest` or `list`, on a channel that stages, public or private, addressed by coordinate or by
//! group.

use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::Presence;

use super::publisher_rig::{ChannelShape, PublisherRig, Set, stranger};
use super::{must, same};
use crate::{ListScope, Publisher};

/// The coordinate at the first place of `staged`, which another writer staged, is seen by no grant
/// of the stage, commit or read phase that names it: not by `observe`, told the set and told none,
/// not by `fetch`, and not by `newest` or `list`. On a public channel a stranger finds nothing of
/// it either.
pub(super) fn another_writers_staged_state_is_seen_by_no_grant<R: PublisherRig>(
    rig: &R,
    shape: &ChannelShape,
    p: &mut R::Publisher,
    staged: &Set,
) {
    let k = &staged.coords[0];
    let blobs = staged.blobs();
    let items = staged.items(&blobs);
    let scope = ListScope {
        channel: k.channel.clone(),
        prefix: String::new(),
    };
    for phase in [GrantPhase::Stage, GrantPhase::Commit, GrantPhase::Read] {
        let grant = rig.grant(p, phase, &staged.coords);
        for (told, set) in [("no set", &[][..]), ("the declared set", &items[..])] {
            same(
                must(
                    p.observe(&grant, k, set),
                    "an observe of the staged state of another writer",
                ),
                Presence::Absent,
                &format!(
                    "the staged state of another writer observed with a {phase:?} grant told {told} ({shape:?})"
                ),
            );
        }
        same(
            must(
                p.fetch(&grant, k),
                "a fetch of the staged state of another writer",
            ),
            None,
            &format!(
                "the bytes of the staged state of another writer fetched with a {phase:?} grant ({shape:?})"
            ),
        );
        let found = must(
            p.newest(&grant, &k.channel, &k.name),
            "a newest of the staged state of another writer",
        );
        same(
            found.iter().all(|(c, _)| c != k),
            true,
            &format!(
                "the staged state of another writer found with newest and a {phase:?} grant ({shape:?})"
            ),
        );
        let listed = must(
            p.list(&grant, &scope),
            "a list of the staged state of another writer",
        );
        same(
            listed.iter().all(|l| l.coordinate != *k),
            true,
            &format!(
                "the staged state of another writer listed with a {phase:?} grant ({shape:?})"
            ),
        );
    }
    if !shape.private {
        same(
            stranger(p, k),
            Presence::Absent,
            "the staged state of another writer seen by a stranger",
        );
    }
}
