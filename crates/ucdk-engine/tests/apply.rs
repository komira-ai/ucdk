// SPDX-License-Identifier: Apache-2.0
//! Apply: creation order, the stamp, idempotence, in-place update, the approved hash, and the
//! ledger records a run leaves.

mod common;

use common::{
    apply, chain, chain_with, done, lowered, mutations, objects, path, paths, physical_of, plan_of,
    planned, records, service_of,
};
use ucdk_engine::{Engine, Outcome, Refusal, Request};
use ucdk_ledger::{Ledger as _, MemoryLedger, Origin, OwnerPath, State};
use ucdk_plan::Action;
use ucdk_provider::{ProviderError, Stamp};
use ucdk_provider_fake::{Fake, FakeConfig, Faults, Op};

fn world() -> (Fake, MemoryLedger) {
    (Fake::standard(), MemoryLedger::new())
}

#[test]
fn apply_creates_in_dependency_order() {
    let (mut fake, ledger) = world();
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert_eq!(
        report.created,
        paths(&["db/run", "api/run", "web/run", "web/public"])
    );
    let creates: Vec<String> = fake
        .calls()
        .iter()
        .filter(|c| c.op == Op::Create)
        .map(|c| c.name.split('~').next().expect("a name").to_owned())
        .collect();
    assert_eq!(creates, ["db-run", "api-run", "web-run", "web-public"]);
}

#[test]
fn apply_stamps_in_the_create_call() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    for (p, record) in records(&ledger) {
        let physical = record.physical.expect("confirmed");
        assert_eq!(
            fake.birth_stamp(&physical),
            Some(Stamp::for_node(&p)),
            "{p} was born with its stamp"
        );
    }
}

#[test]
fn second_apply_is_noop() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let before = objects(&fake);
    let mutated = mutations(&fake);
    let token = ledger.read().expect("read").token;
    let report = done(apply(&mut fake, &ledger, &chain()));
    assert!(report.created.is_empty() && report.updated.is_empty() && report.deleted.is_empty());
    assert_eq!(mutations(&fake), mutated, "no provider mutation");
    assert_eq!(objects(&fake), before);
    assert_eq!(ledger.read().expect("read").token, token, "no ledger write");
}

#[test]
fn a_changed_field_is_updated_in_place() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "db/run");
    let report = done(apply(&mut fake, &ledger, &chain_with(6000, true)));
    assert_eq!(report.updated, paths(&["db/run"]));
    assert!(report.created.is_empty() && report.deleted.is_empty());
    assert_eq!(physical_of(&ledger, "db/run"), id, "same object");
    assert!(objects(&fake).contains("port=6000"));
}

#[test]
fn failed_resource_is_updated_not_recreated() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let id = physical_of(&ledger, "db/run");
    // The update of `db/run` is refused: the object stays as it was and the ledger holds an
    // intent that names it.
    let next = mutations(&fake) as u64 + 1;
    fake.faults_mut().fail_at_call.replace(next);
    let wanted = chain_with(6000, true);
    let Outcome::Failed(failure) = apply(&mut fake, &ledger, &wanted) else {
        panic!("the update should have failed");
    };
    assert_eq!(failure.at, Some(path("db/run")));
    assert_eq!(
        failure.error.to_string(),
        "provider: refused: Rejected".to_owned()
    );
    let stuck = &records(&ledger)[&path("db/run")];
    assert_eq!(
        (stuck.state, stuck.physical.clone()),
        (State::Intent, Some(id.clone()))
    );
    // The next run converges the same object, and creates and deletes nothing.
    fake.clear_calls();
    let report = done(apply(&mut fake, &ledger, &wanted));
    assert_eq!(report.updated, paths(&["db/run"]));
    let ops: Vec<Op> = fake
        .calls()
        .iter()
        .map(|c| c.op)
        .filter(|o| o.mutates())
        .collect();
    assert_eq!(ops, [Op::Update]);
    assert_eq!(physical_of(&ledger, "db/run"), id);
    assert_eq!(records(&ledger)[&path("db/run")].state, State::Confirmed);
}

#[test]
fn ownership_recorded_as_owner_path() {
    let (mut fake, ledger) = world();
    done(apply(&mut fake, &ledger, &chain()));
    let all = records(&ledger);
    assert_eq!(all.len(), 4);
    for (p, record) in &all {
        assert_eq!(record.owner, OwnerPath::of(p), "{p}");
        assert_eq!(record.origin, Origin::Created, "{p}");
        assert_eq!(record.state, State::Confirmed, "{p}");
        let born = fake
            .birth_stamp(record.physical.as_ref().expect("id"))
            .expect("stamp");
        assert_eq!(
            record.owner.top(),
            &born.owner,
            "{p}: the record agrees with the stamp"
        );
    }
    assert_eq!(all[&path("web/public")].owner.to_string(), "web");
    assert_eq!(all[&path("web/public")].name, "web-public");
    assert!(
        all[&path("api/run")].deps.contains(&path("db/run")),
        "edges are recorded"
    );
}

#[test]
fn a_plan_request_changes_nothing() {
    let (mut fake, ledger) = world();
    let plan = planned(plan_of(&mut fake, &ledger, &chain()));
    assert_eq!(plan.steps().len(), 4);
    assert_eq!(
        plan.steps()
            .iter()
            .filter(|s| s.action == Action::Create)
            .count(),
        4
    );
    assert_eq!(fake.object_count(), 0);
    assert!(records(&ledger).is_empty());
    assert!(fake.calls().iter().all(|c| !c.op.mutates()));
}

#[test]
fn a_plan_that_does_not_carry_the_pinned_hash_is_refused() {
    let (mut fake, ledger) = world();
    let plan = planned(plan_of(&mut fake, &ledger, &chain()));
    let other = planned(plan_of(&mut fake, &ledger, &chain_with(1, true)));
    let outcome = Engine::new(&mut fake, &ledger).handle(Request::Apply {
        desired: chain(),
        plan,
        plan_hash: other.hash().clone(),
        rollback: false,
    });
    let Outcome::Refused(refusal) = outcome else {
        panic!("expected a refusal");
    };
    assert!(matches!(refusal, Refusal::PlanHashMismatch { .. }));
    assert!(
        refusal.to_string().starts_with("refused: the plan is "),
        "{refusal}"
    );
    assert_eq!(fake.object_count(), 0);
    assert!(records(&ledger).is_empty());
}

#[test]
fn a_world_that_changed_since_the_approval_is_refused() {
    let (mut fake, ledger) = world();
    let approved = planned(plan_of(&mut fake, &ledger, &chain()));
    // Someone else applies in between: the approved plan now describes the wrong steps.
    done(apply(&mut fake, &ledger, &chain()));
    let mutated = mutations(&fake);
    let hash = approved.hash().clone();
    let outcome = Engine::new(&mut fake, &ledger).handle(Request::Apply {
        desired: chain(),
        plan: approved,
        plan_hash: hash,
        rollback: false,
    });
    let Outcome::Refused(refusal) = outcome else {
        panic!("expected a refusal");
    };
    assert!(matches!(refusal, Refusal::PlanChanged { .. }));
    assert!(refusal.to_string().contains("now"), "{refusal}");
    assert_eq!(mutations(&fake), mutated);
}

#[test]
fn a_foreign_object_refuses_the_whole_run_before_any_change() {
    let mut fake = Fake::new(FakeConfig {
        faults: Faults::default(),
        ..FakeConfig::standard()
    });
    fake.add_foreign(&ucdk_provider::Kind::service(), "api-run", None);
    let ledger = MemoryLedger::new();
    let outcome = apply(&mut fake, &ledger, &chain());
    let Outcome::Refused(refusal) = outcome else {
        panic!("expected a refusal");
    };
    assert!(refusal.to_string().contains("api/run"), "{refusal}");
    assert_eq!(fake.object_count(), 1, "only the foreign object is there");
    assert!(records(&ledger).is_empty());
    assert!(fake.calls().iter().all(|c| !c.op.mutates()));
}

#[test]
fn a_create_refused_before_any_effect_clears_its_intent() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
    fake.faults_mut().fail_at_call.replace(1);
    let Outcome::Failed(failure) = apply(&mut fake, &ledger, &chain()) else {
        panic!("the create should have failed");
    };
    assert_eq!(failure.at, Some(path("db/run")));
    assert!(
        records(&ledger).is_empty(),
        "a refused create leaves no intent"
    );
    assert_eq!(fake.object_count(), 0);
}

#[test]
fn a_create_that_may_have_landed_keeps_its_intent() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
    fake.faults_mut().ambiguous_at_call.replace(1);
    let Outcome::Failed(failure) = apply(&mut fake, &ledger, &chain()) else {
        panic!("the create should have timed out");
    };
    assert_eq!(
        failure.error,
        ucdk_engine::EngineError::Provider(ProviderError::Unknown(
            "the reply timed out".to_owned()
        ))
    );
    let intent = &records(&ledger)[&path("db/run")];
    assert_eq!(
        (intent.state, intent.physical.clone()),
        (State::Intent, None)
    );
    assert_eq!(fake.object_count(), 1);
}

#[test]
fn a_node_that_references_another_resources_output_is_created() {
    use std::collections::BTreeMap;
    use ucdk_proto::v1::r#ref::Output as Arm;
    use ucdk_proto::v1::resource::Body;
    use ucdk_proto::v1::{Output, Ref, Value, value};
    let mut app = service_of("app", 80, false, &[]);
    if let Some(Body::Service(s)) = app.body.as_mut() {
        s.env = BTreeMap::from([(
            "DB".to_owned(),
            Value {
                v: Some(value::V::Ref(Ref {
                    resource: "db".to_owned(),
                    output: Some(Arm::Standard(Output::Url as i32)),
                })),
            },
        )]);
    }
    let wanted = lowered(vec![app, service_of("db", 5432, false, &[])]);
    let (mut fake, ledger) = world();
    let report = done(apply(&mut fake, &ledger, &wanted));
    assert_eq!(report.created, paths(&["db/run", "app/run"]));
}
