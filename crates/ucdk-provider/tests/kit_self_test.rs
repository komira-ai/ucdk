// SPDX-License-Identifier: Apache-2.0
//! The kit must be able to fail. A small conforming provider passes every scenario; the same
//! provider with one defect switched on is flagged by the scenario that guards that
//! behaviour; and a call that fails at any point of any scenario is reported, never passed
//! over.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ucdk_provider::kit::{self, Failure, Harness};
use ucdk_provider::node::digest_of;
use ucdk_provider::{
    CellScope, Created, Deletability, DesiredField, Kind, Live, Node, NodeKey, NodePath,
    Observation, OwnedObject, OwnerScope, PhysicalId, Provider, ProviderError, Refusal, Stamp,
    StampEvidence, StampMode, StateClass, Status, Updated,
};

#[derive(Clone, Copy, Default)]
struct Flaws {
    /// The object is born with no stamp and never gets one.
    skip_stamp: bool,
    /// The stamp is written after the create call, not in it.
    stamp_after: bool,
    /// Create reports the digest of something else.
    wrong_digest: bool,
    /// A create over an existing object replaces it.
    overwrite_existing: bool,
    /// An update of a missing object creates it.
    update_creates: bool,
    /// An update drops the stamp.
    update_drops_stamp: bool,
    /// An update reports success and changes nothing.
    update_ignored: bool,
    /// A delete of an absent object is an error.
    delete_absent_errors: bool,
    /// Reads are answered from the first answer given for that name.
    cached_reads: bool,
    /// A rename takes the stamp away.
    rename_drops_stamp: bool,
    /// A rename gives the object a new physical id.
    rename_new_id: bool,
    /// A foreign object reads as ours.
    foreign_looks_ours: bool,
    /// A foreign object is listed as owned.
    list_includes_foreign: bool,
    /// The object is stamped as another node's.
    stamp_of_another_node: bool,
    /// The unsupported kind is hosted after all.
    hosts_unsupported: bool,
    /// A list returns every stamped object whatever its owner.
    list_ignores_owner: bool,
    /// A delete removes every object of the kind.
    delete_removes_kind: bool,
    /// The stamp mode and the state class change once an object exists.
    declarations_drift: bool,
    /// A fresh object is not deletable.
    fresh_undeletable: bool,
    /// Observe reports a name with the stamp encoded in it.
    encoded_name: bool,
    /// Not a defect: every object holds data the catalog does not model and so cannot be
    /// deleted.
    holds_unmodelled: bool,
    /// Not a defect: a create over a foreign object of the same name lands beside it (the
    /// stamp is part of the physical name) instead of being refused.
    beside_foreign: bool,
    /// The stamp is stored without its extra labels, and the create still succeeds.
    drops_extra: bool,
    /// A failed object is reported as ready.
    ready_for_failed: bool,
    /// A converging object is reported as ready.
    ready_for_converging: bool,
    /// An update leaves a failed object failed.
    update_keeps_failed: bool,
    /// `list_stamped` lists the objects of every cell.
    list_stamped_ignores_cell: bool,
    /// `list_stamped` lists only the objects of one owner.
    list_stamped_one_owner: bool,
    /// `list_stamped` lists a foreign object.
    list_stamped_includes_foreign: bool,
    /// `list_stamped` returns stamps of another scheme.
    list_stamped_other_scheme: bool,
    /// `list_owned` lists the owner's objects of every cell.
    list_owned_ignores_cell: bool,
    /// A hosted kind declares `Embedded`.
    hosted_declares_embedded: bool,
    /// The embedded kind does not declare `Embedded`.
    embedded_undeclared: bool,
    /// A create of the embedded kind lands.
    embedded_creates: bool,
    /// The `k`th call (of any kind, counted from 1) fails with a transient error.
    fail_at: Option<u64>,
}

struct Obj {
    id: PhysicalId,
    kind: Kind,
    name: String,
    fields: Vec<DesiredField>,
    stamp: Option<Stamp>,
    born: Option<Stamp>,
    foreign: bool,
    status: Status,
}

fn embedded_kind() -> Kind {
    Kind::new("grant")
}

struct Stub {
    flaws: Flaws,
    objects: Vec<Obj>,
    calls: u64,
    next: u64,
    cache: Vec<(Kind, String, Observation)>,
    most_calls: Arc<AtomicU64>,
}

impl Stub {
    fn tick(&mut self) -> Result<(), ProviderError> {
        self.calls += 1;
        self.most_calls.fetch_max(self.calls, Ordering::SeqCst);
        if self.flaws.fail_at == Some(self.calls) {
            return Err(ProviderError::Transient("injected".to_owned()));
        }
        Ok(())
    }

    fn insert(&mut self, node: &Node, stamp: Option<Stamp>, born: Option<Stamp>) -> PhysicalId {
        self.next += 1;
        let id = PhysicalId::new(format!("stub-{}", self.next));
        self.objects.push(Obj {
            id: id.clone(),
            kind: node.kind().clone(),
            name: node.name().to_owned(),
            fields: node.fields().to_vec(),
            stamp,
            born,
            foreign: false,
            status: Status::Ready,
        });
        id
    }

    fn live_of(&self, o: &Obj, node: &Node) -> Live {
        let ours = Stamp::for_node(&kit::cell(), node.path());
        let status = match o.status {
            Status::Failed if self.flaws.ready_for_failed => Status::Ready,
            Status::Converging if self.flaws.ready_for_converging => Status::Ready,
            other => other,
        };
        let stamp = match (&o.stamp, o.foreign && self.flaws.foreign_looks_ours) {
            (_, true) => StampEvidence::Present(ours),
            (Some(s), false) => StampEvidence::Present(s.clone()),
            (None, false) => StampEvidence::Absent,
        };
        Live {
            physical: o.id.clone(),
            name: if self.flaws.encoded_name {
                format!("{}~enc", o.name)
            } else {
                o.name.clone()
            },
            stamp,
            status,
            digest: digest_of(&o.kind, &o.fields),
            unmanaged: if self.flaws.holds_unmodelled {
                vec!["data".to_owned()]
            } else {
                Vec::new()
            },
        }
    }
}

impl Provider for Stub {
    fn state_class(&self, _: &Kind) -> StateClass {
        if self.flaws.declarations_drift && !self.objects.is_empty() {
            StateClass::Stateful
        } else {
            StateClass::Stateless
        }
    }

    fn deletability(&self, _: &Kind, live: &Live) -> Deletability {
        if self.flaws.fresh_undeletable || !live.unmanaged.is_empty() {
            Deletability::Undeletable {
                reason: "protected".to_owned(),
            }
        } else {
            Deletability::Deletable
        }
    }

    fn stamp_mode(&self, kind: &Kind) -> StampMode {
        let embedded = *kind == embedded_kind() && !self.flaws.embedded_undeclared;
        if embedded || self.flaws.hosted_declares_embedded {
            StampMode::Embedded
        } else if self.flaws.declarations_drift && !self.objects.is_empty() {
            StampMode::NameEncoded
        } else {
            StampMode::Tag
        }
    }

    fn observe(
        &mut self,
        node: &Node,
        hint: Option<&PhysicalId>,
    ) -> Result<Observation, ProviderError> {
        self.tick()?;
        let cached = self
            .cache
            .iter()
            .find(|(k, n, _)| k == node.kind() && n == node.name())
            .map(|(_, _, o)| o.clone());
        if let (true, Some(old)) = (self.flaws.cached_reads, cached) {
            return Ok(old);
        }
        let found = self.objects.iter().find(|o| match hint {
            Some(id) => &o.id == id,
            None => &o.kind == node.kind() && o.name == node.name(),
        });
        let seen = found.map_or(Observation::Absent, |o| {
            Observation::Present(self.live_of(o, node))
        });
        self.cache
            .push((node.kind().clone(), node.name().to_owned(), seen.clone()));
        Ok(seen)
    }

    fn create(&mut self, node: &Node, stamp: &Stamp) -> Result<Created, ProviderError> {
        self.tick()?;
        if node.kind() == &Kind::job() && !self.flaws.hosts_unsupported {
            return Err(ProviderError::Refused(Refusal::Unsupported));
        }
        if node.kind() == &embedded_kind() && !self.flaws.embedded_creates {
            return Err(ProviderError::Refused(Refusal::Unsupported));
        }
        let beside = self.flaws.beside_foreign;
        let exists = self.objects.iter().position(|o| {
            &o.kind == node.kind() && o.name == node.name() && !(beside && o.foreign)
        });
        let digest = if self.flaws.wrong_digest {
            digest_of(&Kind::new("other"), node.fields())
        } else {
            node.digest()
        };
        if let Some(at) = exists {
            if !self.flaws.overwrite_existing {
                return Err(ProviderError::Refused(Refusal::AlreadyExists));
            }
            let o = &mut self.objects[at];
            o.fields = node.fields().to_vec();
            o.stamp = Some(stamp.clone());
            o.foreign = false;
            return Ok(Created {
                physical: o.id.clone(),
                digest,
            });
        }
        let stripped = Stamp {
            extra: Default::default(),
            ..stamp.clone()
        };
        let stamp = if self.flaws.drops_extra {
            &stripped
        } else {
            stamp
        };
        let other = Stamp::for_node(&kit::cell(), &NodePath::new("other/node").unwrap());
        let stamp = if self.flaws.stamp_of_another_node {
            &other
        } else {
            stamp
        };
        let (stamp_now, born) = match (self.flaws.skip_stamp, self.flaws.stamp_after) {
            (true, _) => (None, None),
            (false, true) => (Some(stamp.clone()), None),
            (false, false) => (Some(stamp.clone()), Some(stamp.clone())),
        };
        let physical = self.insert(node, stamp_now, born);
        Ok(Created { physical, digest })
    }

    fn update(&mut self, node: &Node, live: &Live) -> Result<Updated, ProviderError> {
        self.tick()?;
        let flaws = self.flaws;
        let Some(o) = self.objects.iter_mut().find(|o| o.id == live.physical) else {
            if flaws.update_creates {
                self.insert(node, None, None);
                return Ok(Updated {
                    digest: node.digest(),
                });
            }
            return Err(ProviderError::Refused(Refusal::NotFound));
        };
        if !flaws.update_ignored {
            o.fields = node.fields().to_vec();
        }
        if flaws.update_drops_stamp {
            o.stamp = None;
        }
        if !flaws.update_keeps_failed {
            o.status = Status::Ready;
        }
        Ok(Updated {
            digest: node.digest(),
        })
    }

    fn delete(&mut self, kind: &Kind, physical: &PhysicalId) -> Result<(), ProviderError> {
        self.tick()?;
        let before = self.objects.len();
        if self.flaws.delete_removes_kind {
            self.objects.retain(|o| &o.kind != kind);
        } else {
            self.objects.retain(|o| &o.id != physical);
        }
        if before == self.objects.len() && self.flaws.delete_absent_errors {
            return Err(ProviderError::Permanent("no such object".to_owned()));
        }
        Ok(())
    }

    fn list_owned(&mut self, scope: &OwnerScope) -> Result<Vec<OwnedObject>, ProviderError> {
        self.tick()?;
        let foreign_stamp = Stamp {
            cell: scope.cell.clone(),
            owner: scope.owner.clone(),
            node: NodeKey::of(&NodePath::new("x").unwrap()),
            scheme: 1,
            extra: Default::default(),
        };
        let flaws = self.flaws;
        Ok(self
            .objects
            .iter()
            .filter_map(|o| {
                let stamp = match (&o.stamp, o.foreign && flaws.list_includes_foreign) {
                    (_, true) => Some(foreign_stamp.clone()),
                    (s, false) => s.clone().filter(|s| {
                        (flaws.list_ignores_owner || s.owner == scope.owner)
                            && (flaws.list_owned_ignores_cell || s.cell == scope.cell)
                    }),
                };
                Some(OwnedObject {
                    kind: o.kind.clone(),
                    physical: o.id.clone(),
                    name: o.name.clone(),
                    stamp: stamp?,
                })
            })
            .collect())
    }

    fn list_stamped(&mut self, scope: &CellScope) -> Result<Vec<OwnedObject>, ProviderError> {
        self.tick()?;
        let flaws = self.flaws;
        Ok(self
            .objects
            .iter()
            .filter_map(|o| {
                let stamp = match (&o.stamp, o.foreign && flaws.list_stamped_includes_foreign) {
                    (_, true) => Some(Stamp::for_node(&scope.cell, &NodePath::new("x/y").unwrap())),
                    (s, false) => s.clone().filter(|s| {
                        (flaws.list_stamped_ignores_cell || s.cell == scope.cell)
                            && (!flaws.list_stamped_one_owner || s.owner.as_str() == "kit")
                    }),
                };
                let mut stamp = stamp?;
                if flaws.list_stamped_other_scheme {
                    stamp.scheme = 2;
                }
                Some(OwnedObject {
                    kind: o.kind.clone(),
                    physical: o.id.clone(),
                    name: o.name.clone(),
                    stamp,
                })
            })
            .collect())
    }
}

#[derive(Clone, Default)]
struct StubHarness {
    flaws: Flaws,
    most_calls: Arc<AtomicU64>,
}

impl StubHarness {
    fn with(flaws: Flaws) -> Self {
        Self {
            flaws,
            ..Self::default()
        }
    }
}

impl Harness for StubHarness {
    type Provider = Stub;

    fn fresh(&self) -> Stub {
        Stub {
            flaws: self.flaws,
            objects: Vec::new(),
            calls: 0,
            next: 0,
            cache: Vec::new(),
            most_calls: self.most_calls.clone(),
        }
    }

    fn kinds(&self) -> Vec<Kind> {
        vec![Kind::service()]
    }

    fn unsupported(&self) -> Vec<Kind> {
        vec![Kind::job()]
    }

    fn add_foreign(&self, p: &mut Stub, kind: &Kind, name: &str) -> PhysicalId {
        p.next += 1;
        let id = PhysicalId::new(format!("stub-{}", p.next));
        p.objects.push(Obj {
            id: id.clone(),
            kind: kind.clone(),
            name: name.to_owned(),
            fields: Vec::new(),
            stamp: None,
            born: None,
            foreign: true,
            status: Status::Ready,
        });
        id
    }

    fn set_status(&self, p: &mut Stub, physical: &PhysicalId, status: Status) {
        if let Some(o) = p.objects.iter_mut().find(|o| &o.id == physical) {
            o.status = status;
        }
    }

    fn embedded(&self) -> Vec<Kind> {
        vec![embedded_kind()]
    }

    fn rename(&self, p: &mut Stub, physical: &PhysicalId, new_name: &str) {
        let flaws = self.flaws;
        let next = p.next + 1;
        if let Some(o) = p.objects.iter_mut().find(|o| &o.id == physical) {
            new_name.clone_into(&mut o.name);
            if flaws.rename_drops_stamp {
                o.stamp = None;
            }
            if flaws.rename_new_id {
                o.id = PhysicalId::new(format!("stub-{next}"));
            }
        }
    }

    fn stamp_at_birth(&self, p: &Stub, physical: &PhysicalId) -> Option<Stamp> {
        p.objects
            .iter()
            .find(|o| &o.id == physical)
            .and_then(|o| o.born.clone())
    }
}

type Scenario = fn(&StubHarness) -> Result<(), Failure>;

const SCENARIOS: [(&str, Scenario); 18] = [
    (
        "observe_reports_failed_status",
        kit::observe_reports_failed_status,
    ),
    (
        "observe_reports_converging_status",
        kit::observe_reports_converging_status,
    ),
    (
        "list_stamped_returns_all_owners_skips_foreign",
        kit::list_stamped_returns_all_owners_skips_foreign,
    ),
    (
        "list_stamped_ignores_other_cell",
        kit::list_stamped_ignores_other_cell,
    ),
    (
        "embedded_kinds_are_declared_and_refused",
        kit::embedded_kinds_are_declared_and_refused,
    ),
    (
        "extra_labels_round_trip_or_are_refused",
        kit::extra_labels_round_trip_or_are_refused,
    ),
    (
        "create_stamps_in_create_call",
        kit::create_stamps_in_create_call,
    ),
    (
        "create_existing_is_already_exists",
        kit::create_existing_is_already_exists,
    ),
    (
        "update_missing_is_not_found",
        kit::update_missing_is_not_found,
    ),
    (
        "update_changes_digest_and_keeps_stamp",
        kit::update_changes_digest_and_keeps_stamp,
    ),
    (
        "foreign_same_name_untouched",
        kit::foreign_same_name_untouched,
    ),
    (
        "delete_absent_is_idempotent",
        kit::delete_absent_is_idempotent,
    ),
    ("read_is_live_not_cached", kit::read_is_live_not_cached),
    ("rename_invariance", kit::rename_invariance),
    (
        "delete_removes_only_the_named_object",
        kit::delete_removes_only_the_named_object,
    ),
    (
        "list_owned_filters_by_owner",
        kit::list_owned_filters_by_owner,
    ),
    ("declarations_hold", kit::declarations_hold),
    (
        "declared_unsupported_kinds_refuse",
        kit::declared_unsupported_kinds_refuse,
    ),
];

fn flagged(flaws: Flaws) -> Vec<&'static str> {
    kit::run_all(&StubHarness::with(flaws))
        .into_iter()
        .map(|f| f.scenario)
        .collect()
}

#[test]
fn kit_self_test_flags_nonconforming_stub() {
    // The stub without a defect conforms, and so do two that differ from it in ways the
    // contract allows: creates that land beside a foreign object of the same name, and
    // objects that hold unmodelled data and so cannot be deleted.
    assert_eq!(flagged(Flaws::default()), Vec::<&str>::new());
    let beside = Flaws {
        beside_foreign: true,
        ..Flaws::default()
    };
    assert_eq!(flagged(beside), Vec::<&str>::new());
    let holds_data = Flaws {
        holds_unmodelled: true,
        ..Flaws::default()
    };
    assert_eq!(flagged(holds_data), Vec::<&str>::new());

    // Each defect is flagged by the scenario that guards that behaviour.
    let defects: [(&str, Flaws, &str); 32] = [
        (
            "a failed object reported as ready",
            Flaws {
                ready_for_failed: true,
                ..Flaws::default()
            },
            "observe_reports_failed_status",
        ),
        (
            "an update that leaves the object failed",
            Flaws {
                update_keeps_failed: true,
                ..Flaws::default()
            },
            "observe_reports_failed_status",
        ),
        (
            "a converging object reported as ready",
            Flaws {
                ready_for_converging: true,
                ..Flaws::default()
            },
            "observe_reports_converging_status",
        ),
        (
            "list_stamped across cells",
            Flaws {
                list_stamped_ignores_cell: true,
                ..Flaws::default()
            },
            "list_stamped_ignores_other_cell",
        ),
        (
            "list_owned across cells",
            Flaws {
                list_owned_ignores_cell: true,
                ..Flaws::default()
            },
            "list_stamped_ignores_other_cell",
        ),
        (
            "list_stamped of one owner only",
            Flaws {
                list_stamped_one_owner: true,
                ..Flaws::default()
            },
            "list_stamped_returns_all_owners_skips_foreign",
        ),
        (
            "list_stamped lists a foreign object",
            Flaws {
                list_stamped_includes_foreign: true,
                ..Flaws::default()
            },
            "list_stamped_returns_all_owners_skips_foreign",
        ),
        (
            "list_stamped returns another scheme",
            Flaws {
                list_stamped_other_scheme: true,
                ..Flaws::default()
            },
            "list_stamped_returns_all_owners_skips_foreign",
        ),
        (
            "a hosted kind declares embedded",
            Flaws {
                hosted_declares_embedded: true,
                ..Flaws::default()
            },
            "embedded_kinds_are_declared_and_refused",
        ),
        (
            "an embedded kind that does not declare it",
            Flaws {
                embedded_undeclared: true,
                ..Flaws::default()
            },
            "embedded_kinds_are_declared_and_refused",
        ),
        (
            "an embedded kind that is created",
            Flaws {
                embedded_creates: true,
                ..Flaws::default()
            },
            "embedded_kinds_are_declared_and_refused",
        ),
        (
            "extra labels dropped silently",
            Flaws {
                drops_extra: true,
                ..Flaws::default()
            },
            "extra_labels_round_trip_or_are_refused",
        ),
        (
            "stamped as another node",
            Flaws {
                stamp_of_another_node: true,
                ..Flaws::default()
            },
            "create_stamps_in_create_call",
        ),
        (
            "no stamp",
            Flaws {
                skip_stamp: true,
                ..Flaws::default()
            },
            "create_stamps_in_create_call",
        ),
        (
            "stamp after create",
            Flaws {
                stamp_after: true,
                ..Flaws::default()
            },
            "create_stamps_in_create_call",
        ),
        (
            "wrong digest",
            Flaws {
                wrong_digest: true,
                ..Flaws::default()
            },
            "create_stamps_in_create_call",
        ),
        (
            "overwrite",
            Flaws {
                overwrite_existing: true,
                ..Flaws::default()
            },
            "create_existing_is_already_exists",
        ),
        (
            "update creates",
            Flaws {
                update_creates: true,
                ..Flaws::default()
            },
            "update_missing_is_not_found",
        ),
        (
            "update drops stamp",
            Flaws {
                update_drops_stamp: true,
                ..Flaws::default()
            },
            "update_changes_digest_and_keeps_stamp",
        ),
        (
            "update ignored",
            Flaws {
                update_ignored: true,
                ..Flaws::default()
            },
            "update_changes_digest_and_keeps_stamp",
        ),
        (
            "delete absent errors",
            Flaws {
                delete_absent_errors: true,
                ..Flaws::default()
            },
            "delete_absent_is_idempotent",
        ),
        (
            "cached reads",
            Flaws {
                cached_reads: true,
                ..Flaws::default()
            },
            "read_is_live_not_cached",
        ),
        (
            "rename drops stamp",
            Flaws {
                rename_drops_stamp: true,
                ..Flaws::default()
            },
            "rename_invariance",
        ),
        (
            "rename new id",
            Flaws {
                rename_new_id: true,
                ..Flaws::default()
            },
            "rename_invariance",
        ),
        (
            "foreign looks ours",
            Flaws {
                foreign_looks_ours: true,
                ..Flaws::default()
            },
            "foreign_same_name_untouched",
        ),
        (
            "foreign listed",
            Flaws {
                list_includes_foreign: true,
                ..Flaws::default()
            },
            "foreign_same_name_untouched",
        ),
        (
            "list ignores owner",
            Flaws {
                list_ignores_owner: true,
                ..Flaws::default()
            },
            "list_owned_filters_by_owner",
        ),
        (
            "delete removes the kind",
            Flaws {
                delete_removes_kind: true,
                ..Flaws::default()
            },
            "delete_removes_only_the_named_object",
        ),
        (
            "declarations drift",
            Flaws {
                declarations_drift: true,
                ..Flaws::default()
            },
            "declarations_hold",
        ),
        (
            "fresh object undeletable",
            Flaws {
                fresh_undeletable: true,
                ..Flaws::default()
            },
            "declarations_hold",
        ),
        (
            "encoded name",
            Flaws {
                encoded_name: true,
                ..Flaws::default()
            },
            "declarations_hold",
        ),
        (
            "hosts unsupported",
            Flaws {
                hosts_unsupported: true,
                ..Flaws::default()
            },
            "declared_unsupported_kinds_refuse",
        ),
    ];
    for (what, flaws, scenario) in defects {
        assert!(
            flagged(flaws).contains(&scenario),
            "{what} was not flagged by {scenario}"
        );
    }
}

#[test]
fn a_failing_call_anywhere_in_a_scenario_is_reported() {
    for (name, scenario) in SCENARIOS {
        let mut reached = 0;
        for k in 1..=40 {
            let harness = StubHarness::with(Flaws {
                fail_at: Some(k),
                ..Flaws::default()
            });
            let result = scenario(&harness);
            if harness.most_calls.load(Ordering::SeqCst) >= k {
                reached += 1;
                let failure = result.expect_err(&format!("{name}: call {k} failed and passed"));
                assert_eq!(failure.scenario, name);
                assert!(!failure.problems.is_empty());
            } else {
                assert!(result.is_ok(), "{name}: {result:?}");
            }
        }
        assert!(reached >= 1, "{name} made no call");
    }
}

#[test]
fn a_failure_prints_its_scenario_and_every_problem() {
    let failure = Failure {
        scenario: "s",
        problems: vec!["one".to_owned(), "two".to_owned()],
    };
    assert_eq!(failure.to_string(), "s: one; two");
    let harness = StubHarness::with(Flaws {
        skip_stamp: true,
        ..Flaws::default()
    });
    let text = kit::create_stamps_in_create_call(&harness)
        .unwrap_err()
        .to_string();
    assert!(
        text.starts_with("create_stamps_in_create_call: kind service: "),
        "{text}"
    );
}
