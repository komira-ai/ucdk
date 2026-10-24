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
use crate::observe::{Deletability, Live, Observation, OwnerScope};
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
    /// that carries no stamp of ours, and returns its physical id. `name` is the name the
    /// object has in the cloud as it is: a provider that encodes stamps into physical names
    /// gives the foreign object exactly this name, not an encoded one.
    fn add_foreign(&self, provider: &mut Self::Provider, kind: &Kind, name: &str) -> PhysicalId;

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

/// The node the scenarios use for the `i`th kind: owner `kit`, a distinct name, two fields.
fn sample(kind: &Kind, i: usize, port: i64) -> Node {
    sample_of("kit", kind, i, port)
}

/// As [`sample`], for the owner named `owner`.
fn sample_of(owner: &str, kind: &Kind, i: usize, port: i64) -> Node {
    let path = NodePath::new(&format!("{owner}/n{i}")).expect("the literal path is valid");
    let fields = vec![
        DesiredField::new("port", FieldValue::Int(port)),
        DesiredField::new("label", FieldValue::Text("a".to_owned())),
    ];
    Node::new(path, kind.clone(), format!("{owner}-n{i}"), fields).expect("field names differ")
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
/// never reads as ours and is never listed as owned. A create over it is either refused as
/// `AlreadyExists` (the cloud keeps one name per kind) or lands beside it as a different
/// object (the stamp is part of the physical name); it never changes the foreign object.
pub fn foreign_same_name_untouched<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(h, "foreign_same_name_untouched", |h, kind, i, run| {
        let mut p = h.fresh();
        let node = sample(kind, i, 1);
        let want = Stamp::for_node(node.path());
        let foreign = h.add_foreign(&mut p, kind, node.name());
        // By name the foreign object is found (and is not ours) or, for a provider that
        // looks names up with the stamp in them, not found.
        if let Some(Observation::Present(by_name)) =
            run.call("observe by name", p.observe(&node, None))
        {
            run.check(
                !is_ours(&by_name.stamp, &want),
                "a foreign object reads as ours",
            );
        }
        let Some(before) = run.live("observe by id", p.observe(&node, Some(&foreign))) else {
            return;
        };
        run.check(
            !is_ours(&before.stamp, &want),
            "a foreign object reads as ours",
        );
        match p.create(&node, &want) {
            Ok(created) => run.check(
                created.physical != foreign,
                "create over a foreign object returned the foreign object",
            ),
            Err(e) => run.check(
                e.refusal() == Some(Refusal::AlreadyExists),
                &format!("create over it failed with `{e}`; it must succeed beside it or be refused (AlreadyExists)"),
            ),
        }
        let Some(after) = run.live(
            "observe by id after create",
            p.observe(&node, Some(&foreign)),
        ) else {
            return;
        };
        run.check(after == before, "the foreign object was changed");
        let owned = run.call(
            "list_owned",
            p.list_owned(&OwnerScope::new(node.path().owner())),
        );
        run.check(
            owned.is_some_and(|o| o.iter().all(|o| o.physical != foreign)),
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

/// `list_owned` returns the objects of the scope's owner and nobody else's, and nothing for
/// an owner with no objects.
pub fn list_owned_filters_by_owner<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(h, "list_owned_filters_by_owner", |h, kind, i, run| {
        let mut p = h.fresh();
        let mine = sample_of("kit", kind, i, 1);
        let theirs = sample_of("kit-other", kind, i, 1);
        let mut ids = Vec::new();
        for node in [&mine, &theirs] {
            let want = Stamp::for_node(node.path());
            let Some(created) = run.call("create", p.create(node, &want)) else {
                return;
            };
            ids.push(created.physical);
        }
        for (node, id, other) in [(&mine, &ids[0], &ids[1]), (&theirs, &ids[1], &ids[0])] {
            let scope = OwnerScope::new(node.path().owner());
            let Some(listed) = run.call("list_owned", p.list_owned(&scope)) else {
                return;
            };
            run.check(
                listed.iter().any(|o| &o.physical == id),
                "list_owned omits an object of the owner",
            );
            run.check(
                listed.iter().all(|o| &o.physical != other),
                "list_owned lists another owner's object",
            );
            run.check(
                listed.iter().all(|o| o.stamp.owner == scope.owner),
                "list_owned returns a stamp of another owner",
            );
        }
        let nobody = OwnerScope::new(
            NodePath::new("kit-nobody/x")
                .expect("the literal path is valid")
                .owner(),
        );
        let none = run.call("list_owned of nobody", p.list_owned(&nobody));
        run.check(
            none.is_some_and(|l| l.is_empty()),
            "list_owned lists objects for an owner that has none",
        );
    })
}

/// A delete removes the object it names and no other, whatever its kind and owner.
pub fn delete_removes_only_the_named_object<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(
        h,
        "delete_removes_only_the_named_object",
        |h, kind, i, run| {
            let mut p = h.fresh();
            let doomed = sample_of("kit", kind, i, 1);
            let sibling = sample_of("kit", kind, i + 10, 1);
            let other_owner = sample_of("kit-other", kind, i, 1);
            let mut ids = Vec::new();
            for node in [&doomed, &sibling, &other_owner] {
                let want = Stamp::for_node(node.path());
                let Some(created) = run.call("create", p.create(node, &want)) else {
                    return;
                };
                ids.push(created.physical);
            }
            let bystander = h.add_foreign(&mut p, kind, "kit-bystander");
            run.call("delete", p.delete(kind, &ids[0]));
            run.absent("observe the deleted object", p.observe(&doomed, None));
            for (node, id, what) in [
                (&sibling, &ids[1], "another object of the owner"),
                (&other_owner, &ids[2], "an object of another owner"),
            ] {
                let seen = run.live(what, p.observe(node, Some(id)));
                run.check(
                    seen.is_some_and(|l| is_ours(&l.stamp, &Stamp::for_node(node.path()))),
                    &format!("a delete changed {what}"),
                );
            }
            let foreign = run.live("the foreign object", p.observe(&doomed, Some(&bystander)));
            run.check(
                foreign.is_some_and(|l| !is_ours(&l.stamp, &Stamp::for_node(doomed.path()))),
                "a delete changed the foreign object",
            );
        },
    )
}

/// The provider's declarations about a kind hold: its stamp mode and state class do not
/// change once objects exist, a freshly created object that holds nothing the catalog does
/// not model may be deleted (or `destroy` could never remove what `apply` made), and
/// `observe` reports the human name whatever the stamp mode (a stamp encoded in the physical
/// name stays out of it).
pub fn declarations_hold<H: Harness>(h: &H) -> Result<(), Failure> {
    each_kind(h, "declarations_hold", |h, kind, i, run| {
        let mut p = h.fresh();
        let node = sample(kind, i, 1);
        let want = Stamp::for_node(node.path());
        let (mode, class) = (p.stamp_mode(kind), p.state_class(kind));
        if run.call("create", p.create(&node, &want)).is_none() {
            return;
        }
        run.check(
            p.stamp_mode(kind) == mode,
            "the stamp mode changed once an object existed",
        );
        run.check(
            p.state_class(kind) == class,
            "the state class changed once an object existed",
        );
        let Some(live) = run.live("observe", p.observe(&node, None)) else {
            return;
        };
        run.check(
            live.name == node.name(),
            "observe reports a name other than the human one",
        );
        run.check(
            !live.unmanaged.is_empty() || p.deletability(kind, &live) == Deletability::Deletable,
            "a fresh object holding nothing unmodelled is not deletable",
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
        delete_removes_only_the_named_object(h),
        list_owned_filters_by_owner(h),
        declarations_hold(h),
        read_is_live_not_cached(h),
        rename_invariance(h),
        declared_unsupported_kinds_refuse(h),
    ]
    .into_iter()
    .filter_map(Result::err)
    .collect()
}
