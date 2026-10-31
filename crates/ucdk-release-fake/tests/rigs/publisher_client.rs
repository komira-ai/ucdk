// SPDX-License-Identifier: Apache-2.0
//! The registry as a publisher reaches it. A publisher can see that a grant is for another
//! repository, has expired, is of the wrong phase or does not name the coordinates of a call, and
//! it refuses such a call before it sends anything: the secret of a grant is never presented to a
//! registry it was not issued for. The registry logs every call it is sent before it admits it,
//! so a stub that sends the call and maps the refusal to `Denied` is told by that log.

use std::ops::{Deref, DerefMut};

use ucdk_core::{Clock, ManualClock};
use ucdk_release::codec::GrantFault;
use ucdk_release::publish::{Held, Presence};
use ucdk_release_fake::{FakeRegistry, coordinate_key};
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::kit::LOCATION;
use ucdk_release_seams::{
    CommitAnswer, Grant, ListScope, Listed, Publisher, SeamError, StageAnswer, StageItem,
};

use super::publisher_lax::Defect;
use super::secret::texts;

/// What a call needs of the grant that is presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Needs {
    Stage,
    Commit,
    /// A read: a grant of any phase serves.
    Read,
}

/// The fake registry and the publisher in front of it.
pub struct Client {
    registry: FakeRegistry,
    clock: ManualClock,
    /// A bad grant of this kind is sent all the same: the stub of a publisher that does not look.
    sends: Option<Defect>,
}

impl Deref for Client {
    type Target = FakeRegistry;

    fn deref(&self) -> &FakeRegistry {
        &self.registry
    }
}

impl DerefMut for Client {
    fn deref_mut(&mut self) -> &mut FakeRegistry {
        &mut self.registry
    }
}

impl Client {
    pub(super) fn new(registry: FakeRegistry, clock: ManualClock, sends: Option<Defect>) -> Self {
        Self {
            registry,
            clock,
            sends,
        }
    }

    /// How `g` is bad for a call that needs `needs` and names `ks`, and what refuses it.
    fn refusal(&self, g: &Grant, needs: Needs, ks: &[Coordinate]) -> Option<(Defect, GrantFault)> {
        if g.is_none() {
            return None;
        }
        if g.expires_ms().is_some_and(|e| self.clock.now_millis() >= e) {
            return Some((Defect::Expired, GrantFault::Expired));
        }
        let all = texts(g.secret());
        let scope = all.first().map_or("", String::as_str);
        if all.get(1).map(String::as_str) != Some(LOCATION) {
            return Some((Defect::Foreign, GrantFault::Denied));
        }
        let fits = match needs {
            Needs::Stage => scope == "stage",
            Needs::Commit => scope == "commit",
            Needs::Read => matches!(scope, "stage" | "commit" | "read"),
        };
        if !fits {
            return Some((Defect::OtherWritePhase, GrantFault::Denied));
        }
        let named = all.get(2..).unwrap_or_default();
        ks.iter()
            .any(|k| !named.contains(&coordinate_key(k)))
            .then_some((Defect::Uncovered, GrantFault::Denied))
    }

    /// Refuses a call whose grant is bad before it is sent, unless this is a publisher that sends
    /// it all the same.
    fn check(&self, g: &Grant, needs: Needs, ks: &[Coordinate]) -> Result<(), SeamError> {
        match self.refusal(g, needs, ks) {
            Some((defect, fault)) if self.sends != Some(defect) => {
                Err(SeamError::GrantRefused(fault))
            }
            _ => Ok(()),
        }
    }

    pub(super) fn observe(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        set: &[StageItem<'_>],
    ) -> Result<Presence, SeamError> {
        self.check(g, Needs::Read, std::slice::from_ref(k))?;
        self.registry.observe(g, k, set)
    }

    pub(super) fn stage(
        &mut self,
        g: &Grant,
        items: &[StageItem<'_>],
    ) -> Result<StageAnswer, SeamError> {
        self.check(g, Needs::Stage, &keys_of(items))?;
        self.registry.stage(g, items)
    }

    pub(super) fn abort(&mut self, g: &Grant, ks: &[Coordinate]) -> Result<(), SeamError> {
        self.check(g, Needs::Stage, ks)?;
        self.registry.abort_by_coordinates(g, ks)
    }

    pub(super) fn commit(
        &mut self,
        g: &Grant,
        items: &[StageItem<'_>],
    ) -> Result<CommitAnswer, SeamError> {
        self.check(g, Needs::Commit, &keys_of(items))?;
        self.registry.commit(g, items)
    }

    pub(super) fn yank(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        reason: &str,
    ) -> Result<(), SeamError> {
        self.check(g, Needs::Commit, std::slice::from_ref(k))?;
        self.registry.yank(g, k, reason)
    }

    pub(super) fn fetch(
        &mut self,
        g: &Grant,
        k: &Coordinate,
    ) -> Result<Option<Vec<u8>>, SeamError> {
        self.check(g, Needs::Read, std::slice::from_ref(k))?;
        self.registry.fetch(g, k)
    }

    pub(super) fn newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError> {
        self.check(g, Needs::Read, &[])?;
        self.registry.newest(g, ch, name)
    }

    pub(super) fn list(&mut self, g: &Grant, scope: &ListScope) -> Result<Vec<Listed>, SeamError> {
        self.check(g, Needs::Read, &[])?;
        self.registry.list(g, scope)
    }

    /// How many requests reached the registry, before it admitted them.
    pub(super) fn sent(&self) -> usize {
        self.registry.call_log().len()
    }
}

fn keys_of(items: &[StageItem<'_>]) -> Vec<Coordinate> {
    items.iter().map(|i| i.coordinate.clone()).collect()
}
