// SPDX-License-Identifier: Apache-2.0
//! What the grants of a channel see of the staged state of another writer, which is nothing: a
//! reader of any phase, told the declared set or not, finds nothing of it by `observe`, `fetch`,
//! `newest` or `list`, on a channel that stages, public or private, addressed by coordinate or by
//! group. A stage grant and a commit grant are for `observe` only, so a private channel may refuse
//! their `fetch`, `newest` and `list` with `Denied` instead of answering nothing.

use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::Presence;

use super::publisher_grants::denied;
use super::publisher_rig::{ChannelShape, PublisherRig, Set, stranger};
use super::{must, same};
use crate::{ListScope, Publisher, SeamError};

/// Holds that a read found nothing of staged state: it answered nothing, or, when `may_refuse`,
/// was refused with `Denied`.
pub(super) fn found_nothing<T>(
    answer: Result<T, SeamError>,
    nothing: impl FnOnce(&T) -> bool,
    may_refuse: bool,
    what: &str,
) {
    match answer {
        Ok(found) => same(nothing(&found), true, what),
        Err(refusal) => same(
            may_refuse & denied(&refusal),
            true,
            &format!("{what}: the read was refused with {refusal:?}"),
        ),
    }
}

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
        let may_refuse = shape.private & (phase != GrantPhase::Read);
        found_nothing(
            p.fetch(&grant, k),
            Option::is_none,
            may_refuse,
            &format!(
                "the bytes of the staged state of another writer fetched with a {phase:?} grant ({shape:?})"
            ),
        );
        found_nothing(
            p.newest(&grant, &k.channel, &k.name),
            |found| found.iter().all(|(c, _)| c != k),
            may_refuse,
            &format!(
                "the staged state of another writer found with newest and a {phase:?} grant ({shape:?})"
            ),
        );
        found_nothing(
            p.list(&grant, &scope),
            |listed| listed.iter().all(|l| l.coordinate != *k),
            may_refuse,
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
