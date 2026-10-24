// SPDX-License-Identifier: Apache-2.0
//! The contract kit: plain functions that any [`Provider`] must pass.
//!
//! Each scenario takes a [`Harness`] (which builds a fresh provider and reaches the few
//! things the trait cannot) and returns `Ok(())` or a [`Failure`] naming every way the
//! provider broke the contract. A provider's own test calls the scenarios it needs, or
//! [`run_all`]; a scenario runs once for every kind the harness declares it hosts. A kind
//! the harness declares unsupported is skipped by that declaration and checked only by
//! [`declared_unsupported_kinds_refuse`].

use std::fmt;

use crate::error::{ProviderError, Refusal};
use crate::ids::{NodePath, PhysicalId};
use crate::node::{DesiredField, FieldValue, Kind, Node};
use crate::observe::{Live, Observation, OwnerScope};
use crate::provider::Provider;
use crate::stamp::{Stamp, StampEvidence};

/// What the kit needs from a provider under test beyond the trait.
pub trait Harness {
    type Provider: Provider;

    /// A provider with nothing in it.
    fn fresh(&self) -> Self::Provider;

    /// The kinds this provider hosts: every scenario runs once for each.
    fn kinds(&self) -> Vec<Kind>;

    /// The kinds this provider declares it does not host.
    fn unsupported(&self) -> Vec<Kind>;

    /// Puts an object of `kind` and `name` in the cloud that this cell did not create and
    /// that carries no stamp of ours.
    fn add_foreign(&self, provider: &mut Self::Provider, kind: &Kind, name: &str);

    /// Renames a live object out of band: its human name changes, and whatever stamp it
    /// carries stays.
    fn rename(&self, provider: &mut Self::Provider, physical: &PhysicalId, new_name: &str);

    /// The stamp the object carried in the very call that created it, or `None` when it was
    /// born without one (a stamp written after creation is not a stamp at birth).
    fn stamp_at_birth(&self, provider: &Self::Provider, physical: &PhysicalId) -> Option<Stamp>;
}

/// A scenario that found the provider nonconforming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub scenario: &'static str,
    pub problems: Vec<String>,
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.scenario, self.problems.join("; "))
    }
}

impl std::error::Error for Failure {}

/// Collects what one scenario finds wrong, so a scenario reports all of it, not the first.
struct Run {
    scenario: &'static str,
    kind: String,
    problems: Vec<String>,
}

impl Run {
    fn new(scenario: &'static str) -> Self {
        Self {
            scenario,
            kind: String::new(),
            problems: Vec::new(),
        }
    }

    fn problem(&mut self, text: &str) {
        self.problems.push(format!("kind {}: {text}", self.kind));
    }

    fn check(&mut self, ok: bool, text: &str) {
        if !ok {
            self.problem(text);
        }
    }

    /// A call that must succeed; its error is a problem and the scenario carries on.
    fn call<T>(&mut self, what: &str, result: Result<T, ProviderError>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(e) => {
                self.problem(&format!("{what} failed: {e}"));
                None
            }
        }
    }

    /// A call that must be refused for `want`.
    fn refused<T>(&mut self, what: &str, result: Result<T, ProviderError>, want: Refusal) {
        match result {
            Ok(_) => self.problem(&format!("{what} succeeded; it must be refused ({want:?})")),
            Err(e) => self.check(
                e.refusal() == Some(want),
                &format!("{what} failed with `{e}`; it must be refused ({want:?})"),
            ),
        }
    }

    /// An observation that must find the object.
    fn live(&mut self, what: &str, result: Result<Observation, ProviderError>) -> Option<Live> {
        match self.call(what, result)? {
            Observation::Present(live) => Some(live),
            Observation::Absent => {
                self.problem(&format!("{what}: the object is not there"));
                None
            }
        }
    }

    /// An observation that must not find the object.
    fn absent(&mut self, what: &str, result: Result<Observation, ProviderError>) {
        if let Some(Observation::Present(_)) = self.call(what, result) {
            self.problem(&format!("{what}: the object is still there"));
        }
    }

    fn done(self) -> Result<(), Failure> {
        if self.problems.is_empty() {
            Ok(())
        } else {
            Err(Failure {
                scenario: self.scenario,
                problems: self.problems,
            })
        }
    }
}

/// The node the scenarios use for the `i`th kind: one owner, a distinct name, two fields.
fn sample(kind: &Kind, i: usize, port: i64) -> Node {
    let path = NodePath::new(&format!("kit/n{i}")).expect("the literal path is valid");
    let fields = vec![
        DesiredField::new("port", FieldValue::Int(port)),
        DesiredField::new("label", FieldValue::Text("a".to_owned())),
    ];
    Node::new(path, kind.clone(), format!("kit-n{i}"), fields).expect("field names differ")
}

fn is_ours(evidence: &StampEvidence, want: &Stamp) -> bool {
    matches!(evidence, StampEvidence::Present(s) if s.same_identity(want))
}

/// Runs `body` once for every hosted kind and returns what it found.
fn each_kind<H: Harness>(
    h: &H,
    scenario: &'static str,
    body: impl Fn(&H, &Kind, usize, &mut Run),
) -> Result<(), Failure> {
    let mut run = Run::new(scenario);
    for (i, kind) in h.kinds().iter().enumerate() {
        run.kind = kind.as_str().to_owned();
        body(h, kind, i, &mut run);
    }
    run.done()
}

/// Every object is born carrying its stamp, written in the create call, and reads back as
/// ours with the digest of what was asked for.
pub fn create_stamps_in_create_call<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(h, "create_stamps_in_create_call", |h, kind, i, run| {
        let mut p = h.fresh();
        let node = sample(kind, i, 1);
        let want = Stamp::for_node(node.path());
        let Some(created) = run.call("create", p.create(&node, &want)) else {
            return;
        };
        let born = h.stamp_at_birth(&p, &created.physical);
        run.check(
            born.is_some_and(|s| s.same_identity(&want)),
            "the object was not born carrying the stamp",
        );
        run.check(
            created.digest == node.digest(),
            "create reports a digest other than the node's",
        );
        let Some(live) = run.live("observe", p.observe(&node, None)) else {
            return;
        };
        run.check(
            live.physical == created.physical,
            "observe found another object",
        );
        run.check(
            is_ours(&live.stamp, &want),
            "the stamp does not read back as ours",
        );
        run.check(
            live.digest == node.digest(),
            "observe reports another digest",
        );
    })
}

/// A create over an existing object of the same kind and name is `AlreadyExists`, and the
/// existing object keeps what it had.
pub fn create_existing_is_already_exists<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(h, "create_existing_is_already_exists", |h, kind, i, run| {
        let mut p = h.fresh();
        let first = sample(kind, i, 1);
        let second = sample(kind, i, 2);
        let want = Stamp::for_node(first.path());
        if run.call("create", p.create(&first, &want)).is_none() {
            return;
        }
        run.refused(
            "second create",
            p.create(&second, &want),
            Refusal::AlreadyExists,
        );
        let Some(live) = run.live("observe", p.observe(&first, None)) else {
            return;
        };
        run.check(
            live.digest == first.digest(),
            "the second create changed the object",
        );
    })
}

/// An update of an object that is not there is `NotFound` and creates nothing.
pub fn update_missing_is_not_found<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(h, "update_missing_is_not_found", |h, kind, i, run| {
        let mut p = h.fresh();
        let node = sample(kind, i, 1);
        let ghost = Live {
            physical: PhysicalId::new("kit-ghost"),
            name: node.name().to_owned(),
            stamp: StampEvidence::Absent,
            digest: node.digest(),
            unmanaged: Vec::new(),
        };
        run.refused("update", p.update(&node, &ghost), Refusal::NotFound);
        run.absent("observe", p.observe(&node, None));
    })
}

/// An update changes the digest to the new node's and keeps the stamp and the object.
pub fn update_changes_digest_and_keeps_stamp<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(
        h,
        "update_changes_digest_and_keeps_stamp",
        |h, kind, i, run| {
            let mut p = h.fresh();
            let before = sample(kind, i, 1);
            let after = sample(kind, i, 2);
            let want = Stamp::for_node(before.path());
            let Some(created) = run.call("create", p.create(&before, &want)) else {
                return;
            };
            let Some(live) = run.live("observe", p.observe(&before, None)) else {
                return;
            };
            let Some(updated) = run.call("update", p.update(&after, &live)) else {
                return;
            };
            run.check(
                updated.digest == after.digest(),
                "update reports another digest",
            );
            let Some(now) = run.live("observe after update", p.observe(&after, None)) else {
                return;
            };
            run.check(
                now.digest == after.digest(),
                "the update did not change the object",
            );
            run.check(
                now.physical == created.physical,
                "the update replaced the object",
            );
            run.check(is_ours(&now.stamp, &want), "the update lost the stamp");
        },
    )
}

/// An object this cell did not create, with the node's kind and name, is never overwritten,
/// never reads as ours and is never listed as owned.
pub fn foreign_same_name_untouched<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(h, "foreign_same_name_untouched", |h, kind, i, run| {
        let mut p = h.fresh();
        let node = sample(kind, i, 1);
        let want = Stamp::for_node(node.path());
        h.add_foreign(&mut p, kind, node.name());
        let Some(before) = run.live("observe", p.observe(&node, None)) else {
            return;
        };
        run.check(
            !is_ours(&before.stamp, &want),
            "a foreign object reads as ours",
        );
        run.refused(
            "create over it",
            p.create(&node, &want),
            Refusal::AlreadyExists,
        );
        let Some(after) = run.live("observe after create", p.observe(&node, None)) else {
            return;
        };
        run.check(after == before, "the foreign object was changed");
        let owned = run.call(
            "list_owned",
            p.list_owned(&OwnerScope::new(node.path().owner())),
        );
        run.check(
            owned.is_some_and(|o| o.is_empty()),
            "a foreign object is listed as owned",
        );
    })
}

/// A delete of an object that is not there, and a second delete of one that was, succeed.
pub fn delete_absent_is_idempotent<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(h, "delete_absent_is_idempotent", |h, kind, i, run| {
        let mut p = h.fresh();
        let node = sample(kind, i, 1);
        let want = Stamp::for_node(node.path());
        run.call(
            "delete of a never-created object",
            p.delete(kind, &PhysicalId::new("kit-never")),
        );
        let Some(created) = run.call("create", p.create(&node, &want)) else {
            return;
        };
        run.call("delete", p.delete(kind, &created.physical));
        run.call("second delete", p.delete(kind, &created.physical));
        run.absent("observe", p.observe(&node, None));
    })
}

/// Reads see the cloud as it is now: after every create, update and delete the next read
/// shows it.
pub fn read_is_live_not_cached<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(h, "read_is_live_not_cached", |h, kind, i, run| {
        let mut p = h.fresh();
        let before = sample(kind, i, 1);
        let after = sample(kind, i, 2);
        let scope = OwnerScope::new(before.path().owner());
        let want = Stamp::for_node(before.path());
        run.absent("observe before create", p.observe(&before, None));
        let Some(created) = run.call("create", p.create(&before, &want)) else {
            return;
        };
        let Some(live) = run.live("observe after create", p.observe(&before, None)) else {
            return;
        };
        let listed = run.call("list_owned", p.list_owned(&scope));
        run.check(
            listed.is_some_and(|l| l.iter().any(|o| o.physical == created.physical)),
            "list_owned does not show the new object",
        );
        run.call("update", p.update(&after, &live));
        let seen = run.live("observe after update", p.observe(&after, None));
        run.check(
            seen.is_some_and(|l| l.digest == after.digest()),
            "a read after an update shows the old fields",
        );
        run.call("delete", p.delete(kind, &created.physical));
        run.absent("observe after delete", p.observe(&after, None));
        let listed = run.call("list_owned after delete", p.list_owned(&scope));
        run.check(
            listed.is_some_and(|l| l.is_empty()),
            "list_owned still shows the deleted object",
        );
    })
}

/// A rename out of band changes neither the object's identity nor its ownership: it is
/// still found by its physical id, still ours, and still listed under its owner.
pub fn rename_invariance<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(h, "rename_invariance", |h, kind, i, run| {
        let mut p = h.fresh();
        let node = sample(kind, i, 1);
        let want = Stamp::for_node(node.path());
        let Some(created) = run.call("create", p.create(&node, &want)) else {
            return;
        };
        h.rename(&mut p, &created.physical, "kit-renamed");
        let Some(live) = run.live("observe by hint", p.observe(&node, Some(&created.physical)))
        else {
            return;
        };
        run.check(
            live.physical == created.physical,
            "the hint found another object",
        );
        run.check(live.name == "kit-renamed", "the rename did not show");
        run.check(
            is_ours(&live.stamp, &want),
            "the rename changed who owns the object",
        );
        run.check(
            live.digest == node.digest(),
            "the rename changed the digest",
        );
        let scope = OwnerScope::new(node.path().owner());
        let listed = run.call("list_owned", p.list_owned(&scope));
        run.check(
            listed.is_some_and(|l| l.iter().any(|o| o.physical == created.physical)),
            "the renamed object is no longer listed under its owner",
        );
    })
}

/// A kind the provider declares it does not host is refused before anything is created.
pub fn declared_unsupported_kinds_refuse<H: Harness>(h: &H) -> Result<(), Failure> {
    let mut run = Run::new("declared_unsupported_kinds_refuse");
    for (i, kind) in h.unsupported().iter().enumerate() {
        run.kind = kind.as_str().to_owned();
        let mut p = h.fresh();
        let node = sample(kind, i, 1);
        let want = Stamp::for_node(node.path());
        run.refused("create", p.create(&node, &want), Refusal::Unsupported);
        run.absent("observe", p.observe(&node, None));
    }
    run.done()
}

/// Every scenario, in order; the failures of those that failed.
pub fn run_all<H: Harness>(h: &H) -> Vec<Failure> {
    [
        create_stamps_in_create_call(h),
        create_existing_is_already_exists(h),
        update_missing_is_not_found(h),
        update_changes_digest_and_keeps_stamp(h),
        foreign_same_name_untouched(h),
        delete_absent_is_idempotent(h),
        read_is_live_not_cached(h),
        rename_invariance(h),
        declared_unsupported_kinds_refuse(h),
    ]
    .into_iter()
    .filter_map(Result::err)
    .collect()
}
