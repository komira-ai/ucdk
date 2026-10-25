// SPDX-License-Identifier: Apache-2.0
//! A run that wants a kind the provider does not host is refused whole, before anything is read
//! or written.

mod common;

use common::{cell, done, lowered, plan_of, planned, records, service_of};
use ucdk_engine::{Engine, Mode, Outcome, Refusal, Request};
use ucdk_ledger::MemoryLedger;
use ucdk_proto::v1::resource::Body;
use ucdk_proto::v1::{Job, Resource};
use ucdk_provider::Kind;
use ucdk_provider_fake::{Fake, FakeConfig};

fn job(id: &str) -> Resource {
    let Some(Body::Service(service)) = service_of(id, 80, false, &[]).body else {
        panic!("a service");
    };
    Resource {
        id: id.to_owned(),
        body: Some(Body::Job(Job {
            image: service.image,
            ..Job::default()
        })),
        uses: Vec::new(),
    }
}

fn refusal(outcome: Outcome) -> Vec<(String, Kind)> {
    match outcome {
        Outcome::Refused(Refusal::UnhostedKinds(list)) => {
            list.into_iter().map(|(p, k)| (p.to_string(), k)).collect()
        }
        other => panic!("not an unhosted refusal: {other:?}"),
    }
}

/// Proves: with a hosted resource that sorts first and an unhosted one after it, plan, apply,
/// status and apply under a good hash are all refused naming only the unhosted
/// node, and the provider is never called and the ledger stays empty. Catches a run that
/// creates the hosted resource and then fails on the other.
#[test]
fn an_unhosted_kind_is_refused_before_any_call() {
    let both = lowered(vec![service_of("a-svc", 80, false, &[]), job("z-job")]);
    let mut limited = Fake::new(FakeConfig::limited());
    let ledger = MemoryLedger::new();

    // A plan a hosting provider made, so the hash is a good one.
    let mut hosting = Fake::standard();
    let plan = planned(plan_of(&mut hosting, &MemoryLedger::new(), &both));
    let hash = plan.hash().clone();

    let mut engine = Engine::new(&mut limited, &ledger, cell());
    let wanted = vec![("z-job/run".to_owned(), Kind::job())];
    assert_eq!(
        refusal(engine.handle(Request::Plan {
            desired: both.clone(),
            mode: Mode::Apply
        })),
        wanted
    );
    assert_eq!(
        refusal(engine.handle(Request::Apply {
            desired: both.clone(),
            plan,
            plan_hash: hash,
            rollback: false
        })),
        wanted
    );
    assert_eq!(
        refusal(engine.handle(Request::Status { desired: both })),
        wanted
    );
    assert!(limited.calls().is_empty(), "nothing was read or written");
    assert!(records(&ledger).is_empty());
}

/// Proves: the refusal is for what the list wants. A destroy of a list that names an unhosted
/// kind is not refused for it, so what was applied can still be removed. Catches a check that also refuses a destroy.
#[test]
fn a_destroy_is_not_refused_for_an_unhosted_kind() {
    let hosted = lowered(vec![service_of("a-svc", 80, false, &[])]);
    let mut limited = Fake::new(FakeConfig::limited());
    let ledger = MemoryLedger::new();
    done(common::apply(&mut limited, &ledger, &hosted));
    let with_job = lowered(vec![service_of("a-svc", 80, false, &[]), job("z-job")]);
    let report = done(common::destroy(&mut limited, &ledger, &with_job));
    assert_eq!(report.deleted.len(), 1, "{report:?}");
    assert!(records(&ledger).is_empty());
}
