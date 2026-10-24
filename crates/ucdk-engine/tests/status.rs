// SPDX-License-Identifier: Apache-2.0
//! Status and cell: a failed object is updated, a converging one is left alone and not
//! confirmed until it is ready, and an object stamped for another cell is not ours.

mod common;

use common::{
    Hooked, apply, cell, chain, chain_with, done, mutations, objects, path, paths, physical_of,
    plan_of, planned, records,
};
use ucdk_engine::{Engine, Mode, Outcome, Refusal, Request};
use ucdk_ledger::{Ledger as _, MemoryLedger, State};
use ucdk_plan::{Action, ConflictKind};
use ucdk_provider::{
    CellScope, Created, Deletability, Kind, Live, Node, Observation, OwnedObject, OwnerScope,
    PhysicalId, Provider, ProviderError, Stamp, StampMode, StateClass, Status, Updated, kit,
};
use ucdk_provider_fake::{Fake, FakeConfig, Faults, Op};

fn world() -> (Fake, MemoryLedger) {
    (Fake::standard(), MemoryLedger::new())
}

fn state_of(ledger: &MemoryLedger, p: &str) -> State {
    records(ledger)[&path(p)].state
}

fn actions(plan: &ucdk_plan::Plan) -> Vec<(String, Action)> {
    plan.steps()
        .iter()
        .map(|s| (s.path.to_string(), s.action.clone()))
        .collect()
}

#[test]
fn failed_object_is_updated_not_noop() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "db/run");
    assert!(fake.set_status(&id, Status::Failed));
    let before = objects(&fake);
    assert!(before.contains("status=failed"));

    // The fields match; only the status says the object is not working. The plan updates it.
    let plan = planned(plan_of(&mut fake, &ledger, &chain()));
    let updates: Vec<String> = actions(&plan)
        .into_iter()
        .filter(|(_, a)| *a == Action::Update)
        .map(|(p, _)| p)
        .collect();
    assert_eq!(updates, ["db/run"]);

    fake.clear_calls();
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(report.updated, paths(&["db/run"]));
    assert!(report.converging.is_empty());
    let mutating: Vec<(Op, Option<PhysicalId>)> = fake
        .calls()
        .iter()
        .filter(|c| c.op.mutates())
        .map(|c| (c.op, c.physical.clone()))
        .collect();
    assert_eq!(
        mutating,
        [(Op::Update, Some(id))],
        "one update, nothing else"
    );
    assert!(objects(&fake).contains("status=ready"));
    assert!(!objects(&fake).contains("status=failed"));
    assert_eq!(state_of(&ledger, "db/run"), State::Confirmed);

    // Repaired: the next run has nothing to do.
    let writes = mutations(&fake);
    let again = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(again.updated.len() + again.created.len(), 0);
    assert_eq!(mutations(&fake), writes);
}

#[test]
fn converging_not_confirmed_until_ready() {
    let mut fake = Fake::new(FakeConfig {
        faults: Faults {
            converging_reads: 12,
            ..Faults::default()
        },
        ..FakeConfig::standard()
    });
    let ledger = MemoryLedger::new();
    let first = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(first.created.len(), 4);
    assert_eq!(
        first.converging, first.created,
        "every new object is still coming up"
    );
    for p in ["db/run", "api/run", "web/run", "web/public"] {
        let record = &records(&ledger)[&path(p)];
        assert_eq!(record.state, State::Intent, "{p} is not confirmed");
        assert!(
            record.physical.is_some(),
            "{p}: the intent names the object"
        );
    }

    // While the objects converge, a run makes no write and confirms nothing that is not
    // ready. A plan that was made while an object converged and is applied after it became
    // ready is refused as changed, which is the same: no write.
    let writes = mutations(&fake);
    let mut resumed = Vec::new();
    let mut rounds_waiting = 0;
    for round in 1..=30 {
        match apply(&mut fake, &ledger, &chain()) {
            Outcome::Done(report) => {
                assert!(
                    report.updated.is_empty() && report.created.is_empty(),
                    "round {round}: {report:?}"
                );
                if !report.converging.is_empty() {
                    rounds_waiting += 1;
                }
                resumed.extend(report.resumed);
            }
            Outcome::Refused(Refusal::PlanChanged { .. }) => rounds_waiting += 1,
            other => panic!("round {round}: {other:?}"),
        }
        assert_eq!(mutations(&fake), writes, "round {round}: no write");
        if records(&ledger)
            .values()
            .all(|r| r.state == State::Confirmed)
        {
            break;
        }
    }
    assert!(
        rounds_waiting >= 1,
        "the objects stayed converging for at least one run"
    );
    resumed.sort();
    assert_eq!(
        resumed,
        paths(&["api/run", "db/run", "web/public", "web/run"]),
        "each record is confirmed by finding its object in place, with no create"
    );
    assert!(
        records(&ledger)
            .values()
            .all(|r| r.state == State::Confirmed)
    );
}

#[test]
fn a_converging_object_is_left_alone_even_when_its_fields_differ() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "db/run");
    assert!(fake.set_status(&id, Status::Converging));
    let writes = mutations(&fake);

    let wanted = chain_with(6000, true);
    let plan = planned(plan_of(&mut fake, &ledger, &wanted));
    assert!(actions(&plan).contains(&("db/run".to_owned(), Action::Converging)));
    let report = done(apply(&mut fake, &ledger, &wanted));
    assert_eq!(report.converging, paths(&["db/run"]));
    assert!(!report.updated.contains(&path("db/run")));
    assert!(
        objects(&fake).contains("port=5432"),
        "the object is as it was"
    );
    assert!(!objects(&fake).contains("port=6000"));
    assert_eq!(mutations(&fake), writes);
    assert_eq!(
        state_of(&ledger, "db/run"),
        State::Confirmed,
        "nothing was rewritten"
    );
}

#[test]
fn an_object_that_became_ready_after_the_plan_refuses_the_approved_plan() {
    // The plan said Converging and the hash pins that: a run that finds the object ready is
    // refused as a changed plan, never carried out under a stale approval.
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "db/run");
    assert!(fake.set_status(&id, Status::Converging));
    let waiting = planned(plan_of(&mut fake, &ledger, &chain()));
    let hash = waiting.hash().clone();
    assert!(fake.set_status(&id, Status::Ready));
    let outcome = Engine::new(&mut fake, &ledger, cell()).handle(Request::Apply {
        desired: chain(),
        plan: waiting,
        plan_hash: hash,
        rollback: false,
    });
    assert!(matches!(
        outcome,
        Outcome::Refused(Refusal::PlanChanged { .. })
    ));
}

#[test]
fn an_object_born_failed_keeps_its_intent_and_is_updated_by_the_next_run() {
    let mut fake = Fake::new(FakeConfig {
        faults: Faults {
            create_failed: true,
            ..Faults::default()
        },
        ..FakeConfig::standard()
    });
    let ledger = MemoryLedger::new();
    let first = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(first.created.len(), 4);
    assert!(first.converging.is_empty(), "failed is not converging");
    for p in ["db/run", "api/run", "web/run", "web/public"] {
        assert_eq!(state_of(&ledger, p), State::Intent, "{p} is not confirmed");
    }
    *fake.faults_mut().create_failed = false;
    let second = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(second.updated.len(), 4, "each failed object is updated");
    assert!(second.created.is_empty());
    for p in ["db/run", "api/run", "web/run", "web/public"] {
        assert_eq!(state_of(&ledger, p), State::Confirmed, "{p}");
    }
}

/// The fake, except that an update leaves the object in `after` (or deletes it).
struct Relapse<'f> {
    fake: &'f mut Fake,
    after: Option<Status>,
}

impl Provider for Relapse<'_> {
    fn state_class(&self, kind: &Kind) -> StateClass {
        self.fake.state_class(kind)
    }

    fn deletability(&self, kind: &Kind, live: &Live) -> Deletability {
        self.fake.deletability(kind, live)
    }

    fn stamp_mode(&self, kind: &Kind) -> StampMode {
        self.fake.stamp_mode(kind)
    }

    fn observe(
        &mut self,
        node: &Node,
        hint: Option<&PhysicalId>,
    ) -> Result<Observation, ProviderError> {
        self.fake.observe(node, hint)
    }

    fn create(&mut self, node: &Node, stamp: &Stamp) -> Result<Created, ProviderError> {
        self.fake.create(node, stamp)
    }

    fn update(&mut self, node: &Node, live: &Live) -> Result<Updated, ProviderError> {
        let done = self.fake.update(node, live)?;
        match self.after {
            Some(status) => {
                self.fake.set_status(&live.physical, status);
            }
            None => self.fake.delete(node.kind(), &live.physical)?,
        }
        Ok(done)
    }

    fn delete(&mut self, kind: &Kind, physical: &PhysicalId) -> Result<(), ProviderError> {
        self.fake.delete(kind, physical)
    }

    fn list_owned(&mut self, scope: &OwnerScope) -> Result<Vec<OwnedObject>, ProviderError> {
        self.fake.list_owned(scope)
    }

    fn list_stamped(&mut self, scope: &CellScope) -> Result<Vec<OwnedObject>, ProviderError> {
        self.fake.list_stamped(scope)
    }
}

#[test]
fn a_repaired_object_is_confirmed_only_if_it_reads_ready_afterwards() {
    for (after, converging) in [
        (Some(Status::Converging), true),
        (Some(Status::Failed), false),
        (None, false),
    ] {
        let (mut fake, ledger) = world();
        done(apply(&mut fake, &ledger, &chain()));
        let id = physical_of(&ledger, "db/run");
        assert!(fake.set_status(&id, Status::Failed));
        let mut relapse = Relapse {
            fake: &mut fake,
            after,
        };
        let report = done(apply(&mut relapse, &ledger, &chain()));
        assert_eq!(report.updated, paths(&["db/run"]), "{after:?}");
        assert_eq!(
            report.converging == paths(&["db/run"]),
            converging,
            "{after:?}"
        );
        assert_eq!(
            state_of(&ledger, "db/run"),
            State::Intent,
            "{after:?}: not confirmed"
        );
        assert_eq!(records(&ledger)[&path("db/run")].physical, Some(id));
    }
}

#[test]
fn a_kind_with_no_stamp_of_its_own_is_refused_before_anything_is_read() {
    let ledger = MemoryLedger::new();
    let mut fake = Fake::new(FakeConfig {
        embedded: [Kind::service()].into(),
        ..FakeConfig::standard()
    });
    for mode in [Mode::Apply, Mode::Destroy] {
        let outcome = Engine::new(&mut fake, &ledger, cell()).handle(Request::Plan {
            desired: chain(),
            mode,
        });
        let Outcome::Refused(refusal) = outcome else {
            panic!("not refused: {outcome:?}");
        };
        let Refusal::UnsupportedStampMode(list) = &refusal else {
            panic!("wrong refusal: {refusal:?}");
        };
        assert!(!list.is_empty(), "the nodes of the kind are named");
        assert!(list.len() <= 4);
        assert!(
            refusal.to_string().starts_with(
                "refused: kinds that carry no stamp of their own are not applied yet: "
            )
        );
        assert!(refusal.to_string().contains("db/run"));
    }
    // A plan made for a provider that hosts them is not run against one that does not.
    let (mut plain, other) = world();
    let plan = planned(plan_of(&mut plain, &other, &chain()));
    let hash = plan.hash().clone();
    let outcome = Engine::new(&mut fake, &ledger, cell()).handle(Request::Apply {
        desired: chain(),
        plan,
        plan_hash: hash,
        rollback: false,
    });
    assert!(matches!(
        outcome,
        Outcome::Refused(Refusal::UnsupportedStampMode(_))
    ));
    assert!(fake.calls().is_empty(), "nothing was read or written");
    assert!(ledger.read().unwrap().records.is_empty());
}

#[test]
fn every_stamp_an_apply_writes_names_the_cell() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    for record in records(&ledger).values() {
        let born = fake
            .birth_stamp(record.physical.as_ref().expect("confirmed"))
            .expect("a stamp at birth");
        assert_eq!(born.cell, cell());
    }
}

#[test]
fn an_object_of_another_cell_is_not_ours() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let writes = mutations(&fake);
    let outcome = Engine::new(&mut fake, &ledger, kit::other_cell()).handle(Request::Plan {
        desired: chain(),
        mode: Mode::Apply,
    });
    let Outcome::Refused(Refusal::Conflicts(conflicts)) = outcome else {
        panic!("not refused: {outcome:?}");
    };
    assert_eq!(conflicts.len(), 4);
    assert!(conflicts.iter().all(|c| c.why == ConflictKind::OtherOwner));
    assert_eq!(mutations(&fake), writes);
}

#[test]
fn an_object_that_starts_converging_after_the_plan_is_not_written() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "db/run");
    let writes = mutations(&fake);
    // A plan reads each node once, and the run reads them all again before its first step.
    // Just before that step re-reads `db/run` (the read after both) it starts converging.
    let wanted = chain_with(6000, true);
    let mut counting = Hooked::new(&mut fake, usize::MAX, |_| {});
    planned(plan_of(&mut counting, &ledger, &wanted));
    let reads_per_plan = counting.calls();
    drop(counting);
    let mut hooked = Hooked::new(&mut fake, 2 * reads_per_plan + 1, {
        let id = id.clone();
        move |fake| {
            assert!(fake.set_status(&id, Status::Converging));
        }
    });
    let report = done(apply(&mut hooked, &ledger, &wanted));
    drop(hooked);
    assert_eq!(report.converging, paths(&["db/run"]));
    assert!(report.updated.is_empty(), "{report:?}");
    assert_eq!(mutations(&fake), writes, "no write to a converging object");
    assert!(objects(&fake).contains("port=5432"));
}
