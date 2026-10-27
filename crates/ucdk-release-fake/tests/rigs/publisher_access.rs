// SPDX-License-Identifier: Apache-2.0
//! The stubs of the publisher that are wrong about a grant: the grants they take for good ones,
//! and the helpers that write a grant as a lax registry would read it.

use ucdk_release::codec::GrantPhase;
use ucdk_release_fake::coordinate_key;
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::kit::LOCATION;
use ucdk_release_seams::{Grant, Listed, StageItem};

use super::publisher::Flawed;
use super::publisher_flaw::{Bad, Call, Flaw};
use super::publisher_grant::scope_of;
use super::secret::{framed, texts};

/// The phase a write needs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Needs {
    Stage,
    Commit,
}

/// The grant `g` with its texts and expiry changed by `change`.
pub(super) fn rewritten(g: &Grant, change: impl FnOnce(&mut Vec<String>, &mut u64)) -> Grant {
    let mut all = texts(g.secret());
    let mut expires = g.expires_ms().unwrap_or(u64::MAX);
    change(&mut all, &mut expires);
    Grant::issued(framed(&all), expires)
}

/// Whether `g` names the last coordinate of `items`.
pub(super) fn covers_last(g: &Grant, items: &[StageItem<'_>]) -> bool {
    let Some(last) = items.last() else {
        return false;
    };
    let key = coordinate_key(last.coordinate);
    texts(g.secret()).iter().skip(2).any(|text| *text == key)
}

/// The grant `g` as if it had been issued for every coordinate of `items`: the same scope,
/// repository and expiry.
pub(super) fn covering_every(g: &Grant, items: &[StageItem<'_>]) -> Grant {
    rewritten(g, |all, _| {
        all.truncate(2);
        all.extend(items.iter().map(|item| coordinate_key(item.coordinate)));
    })
}

impl Flawed {
    /// Whether `g` is no grant on a private channel and `flaw` is the flaw of the stub.
    pub(super) fn lets_a_stranger_in(&self, flaw: Flaw, g: &Grant) -> bool {
        self.is(flaw) && self.shape.private && g.is_none()
    }

    /// The grant a stub that lets a stranger write takes `g` for, when `g` is no grant and
    /// `flaw` is the flaw of the stub.
    pub(super) fn writer_for_a_stranger(
        &self,
        flaw: Flaw,
        g: &Grant,
        phase: GrantPhase,
        coordinates: &[Coordinate],
    ) -> Option<Grant> {
        (self.is(flaw) && g.is_none()).then(|| self.internal(phase, coordinates))
    }

    /// The grant a private channel of a stub takes a read grant `g` for when it is lax about it:
    /// every read for another repository's or an expired grant, and, for a read of `k`, one that
    /// does not name `k`.
    pub(super) fn lax_read(&self, g: &Grant, k: Option<&Coordinate>) -> Option<Grant> {
        if !self.shape.private || g.is_none() {
            return None;
        }
        match (self.flaw, k) {
            (Some(Flaw::PrivateTakesAForeignReadGrant), _) => Some(rewritten(g, |all, _| {
                LOCATION.clone_into(&mut all[1]);
            })),
            (Some(Flaw::PrivateTakesAnExpiredReadGrant), _) => {
                Some(rewritten(g, |_, expires| *expires = u64::MAX))
            }
            (Some(Flaw::PrivateTakesAnUncoveredReadGrant), Some(k)) => {
                Some(rewritten(g, |all, _| {
                    all.truncate(2);
                    all.push(coordinate_key(k));
                }))
            }
            _ => None,
        }
    }

    /// The grant the stub takes `g` for in a call of the kind `call` when `g` is bad in the way
    /// the stub is lax about and the stub is lax about that call.
    pub(super) fn lax_write(
        &self,
        g: &Grant,
        call: Call,
        coordinates: &[Coordinate],
    ) -> Option<Grant> {
        let Some(Flaw::BadGrantTakenBy(bad, lax_call)) = self.flaw else {
            return None;
        };
        if lax_call != call || g.is_none() {
            return None;
        }
        Some(match bad {
            Bad::Foreign => rewritten(g, |all, _| {
                LOCATION.clone_into(&mut all[1]);
            }),
            Bad::Expired => rewritten(g, |_, expires| *expires = u64::MAX),
            Bad::Uncovered => rewritten(g, |all, _| {
                all.truncate(2);
                all.extend(coordinates.iter().map(coordinate_key));
            }),
        })
    }

    /// The grant a flawed registry would take `g` for, when it takes it for another.
    pub(super) fn regrant(&self, g: &Grant, needs: Needs) -> Option<Grant> {
        if g.is_none() {
            return None;
        }
        let scope = scope_of(g);
        let wanted = if needs == Needs::Stage {
            "stage"
        } else {
            "commit"
        };
        match self.flaw {
            Some(Flaw::AForeignGrantIsAccepted) => Some(rewritten(g, |all, _| {
                LOCATION.clone_into(&mut all[1]);
            })),
            Some(Flaw::TheWrongPhaseIsAccepted) if scope != wanted && scope != "read" => {
                Some(rewritten(g, |all, _| wanted.clone_into(&mut all[0])))
            }
            Some(Flaw::AnExpiredGrantIsAccepted) => {
                Some(rewritten(g, |_, expires| *expires = u64::MAX))
            }
            Some(Flaw::AReadGrantMayWrite) if scope == "read" => {
                Some(rewritten(g, |all, _| wanted.clone_into(&mut all[0])))
            }
            _ => None,
        }
    }

    /// The staged coordinates of the channel `ch` whose name starts with `prefix`, as listed.
    pub(super) fn staged_listed(&mut self, ch: &str, prefix: &str) -> Vec<Listed> {
        let staged = self.known_where(|me, c| {
            me.inner.is_staged(c) && c.channel == ch && c.name.starts_with(prefix)
        });
        staged
            .into_iter()
            .filter_map(|coordinate| {
                self.held_now(&coordinate)
                    .map(|held| Listed { coordinate, held })
            })
            .collect()
    }
}
