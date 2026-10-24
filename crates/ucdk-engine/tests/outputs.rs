// SPDX-License-Identifier: Apache-2.0
//! Recorded outputs and the nodes that read them.

mod common;

use std::collections::BTreeMap;

use common::{
    Hooked, apply, bound_pair, done, lowered, mutations, objects, path, physical_of, plan_of,
    planned, records, service_of, service_reading,
};
use ucdk_engine::{EngineError, Outcome};
use ucdk_ledger::{MemoryLedger, State};
use ucdk_plan::Action;
use ucdk_proto::v1::Output;
use ucdk_provider::Kind;
use ucdk_provider_fake::{Fake, FakeConfig, Faults};

const DB_URL: &str = "https://db-run.example.org";

fn actions(plan: &ucdk_plan::Plan) -> Vec<(&str, &Action)> {
    plan.steps()
        .iter()
        .map(|s| (s.path.as_str(), &s.action))
        .collect()
}

#[test]
fn second_apply_with_ref_is_noop() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
    done(apply(&mut fake, &ledger, &bound_pair()));
    let mutated = mutations(&fake);
    let plan = planned(plan_of(&mut fake, &ledger, &bound_pair()));
    assert!(
        plan.steps().iter().all(|s| s.action == Action::Noop),
        "{:?}",
        plan.steps()
    );
    let report = done(apply(&mut fake, &ledger, &bound_pair()));
    assert!(report.created.is_empty() && report.updated.is_empty());
    assert_eq!(mutations(&fake), mutated, "the second run wrote nothing");
}

#[test]
fn first_apply_resolves_the_reference_once_the_producer_is_confirmed() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
    let plan = planned(plan_of(&mut fake, &ledger, &bound_pair()));
    assert_eq!(
        actions(&plan),
        [
            ("db/run", &Action::Create),
            ("app/run", &Action::KnownAfterApply)
        ]
    );
    let report = done(apply(&mut fake, &ledger, &bound_pair()));
    assert_eq!(report.created, [path("db/run"), path("app/run")]);
    // The consumer was created with the producer's value, not without it.
    let dump = objects(&fake);
    let line = dump
        .lines()
        .find(|l| l.contains("app-run"))
        .expect("the consumer is there");
    assert!(
        line.contains("env.DB_URL") && line.contains(DB_URL),
        "{line}"
    );
}

#[test]
fn outputs_survive_reread() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
    done(apply(&mut fake, &ledger, &bound_pair()));
    let expected = BTreeMap::from([
        ("HOST".to_owned(), "db-run.example.org".to_owned()),
        ("URL".to_owned(), DB_URL.to_owned()),
    ]);
    // Read twice: what is recorded is in the document, not held by the engine that wrote it.
    for _ in 0..2 {
        let held = records(&ledger);
        assert_eq!(held[&path("db/run")].outputs, expected);
        assert_eq!(held[&path("app/run")].outputs.len(), 2);
    }
}

#[test]
fn secret_output_never_written_to_ledger() {
    let mut fake = Fake::new(FakeConfig {
        secret_output: [Kind::service()].into(),
        ..FakeConfig::standard()
    });
    let ledger = MemoryLedger::new();
    done(apply(&mut fake, &ledger, &bound_pair()));
    for (p, record) in records(&ledger) {
        assert!(
            !record.outputs.contains_key("TOKEN"),
            "{p} holds a secret output"
        );
        assert!(record.outputs.contains_key("URL"), "{p} keeps its URL");
    }
    let document = format!("{:?}", ledger_document(&ledger));
    assert!(!document.contains("token-of-"), "{document}");
}

fn ledger_document(ledger: &MemoryLedger) -> impl std::fmt::Debug {
    use ucdk_ledger::Ledger as _;
    ledger.read().expect("read")
}

#[test]
fn a_changed_producer_plans_its_consumer_after_apply() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
    done(apply(&mut fake, &ledger, &bound_pair()));
    let changed = lowered(vec![
        service_reading("app", 80, "DB_URL", "db", Output::Url),
        service_of("db", 5433, false, &[]),
    ]);
    let plan = planned(plan_of(&mut fake, &ledger, &changed));
    assert_eq!(
        actions(&plan),
        [
            ("db/run", &Action::Update),
            ("app/run", &Action::KnownAfterApply)
        ]
    );
    // The producer is updated; its URL is the same, so the consumer is found as it should be.
    let report = done(apply(&mut fake, &ledger, &changed));
    assert_eq!(report.updated, [path("db/run")]);
    assert!(report.created.is_empty());
}

#[test]
fn a_consumer_waits_while_its_producer_is_coming_up() {
    let mut fake = Fake::new(FakeConfig {
        faults: Faults {
            converging_reads: 6,
            ..Faults::default()
        },
        ..FakeConfig::standard()
    });
    let ledger = MemoryLedger::new();
    let first = done(apply(&mut fake, &ledger, &bound_pair()));
    assert_eq!(
        first.created,
        [path("db/run")],
        "the consumer is not created"
    );
    assert_eq!(first.converging, [path("db/run"), path("app/run")]);
    let writes = mutations(&fake);
    for round in 1..=30 {
        match apply(&mut fake, &ledger, &bound_pair()) {
            Outcome::Done(_) | Outcome::Refused(_) => {}
            other => panic!("round {round}: {other:?}"),
        }
        if records(&ledger)
            .values()
            .all(|r| r.state == State::Confirmed)
            && records(&ledger).len() == 2
        {
            break;
        }
    }
    assert!(mutations(&fake) > writes, "the consumer was created later");
    let held = records(&ledger);
    assert_eq!(held[&path("app/run")].state, State::Confirmed);
    assert_eq!(held[&path("db/run")].outputs["URL"], DB_URL);
    assert!(objects(&fake).contains(DB_URL));
}

#[test]
fn an_output_the_producer_does_not_report_stops_the_run() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
    let mut silent = Hooked::silent(&mut fake);
    let Outcome::Failed(failure) = apply(&mut silent, &ledger, &bound_pair()) else {
        panic!("the run should have failed");
    };
    let expected = EngineError::OutputUnknown {
        path: path("app/run"),
        resource: "db".to_owned(),
        output: "URL".to_owned(),
    };
    assert_eq!(failure.error, expected);
    assert_eq!(failure.at, Some(path("app/run")));
    assert_eq!(
        expected.to_string(),
        "app/run: `db` is applied and reports no URL for it to read"
    );
    drop(silent);
    assert!(
        !records(&ledger).contains_key(&path("app/run")),
        "nothing was created for the consumer"
    );
}

#[test]
fn a_consumer_does_not_read_the_outputs_of_an_unconfirmed_update() {
    let mut fake = Fake::standard();
    let ledger = MemoryLedger::new();
    done(apply(&mut fake, &ledger, &bound_pair()));
    let db = physical_of(&ledger, "db/run");
    assert!(fake.set_status(&db, ucdk_provider::Status::Failed));
    let changed = lowered(vec![
        service_reading("app", 99, "DB_URL", "db", Output::Url),
        service_of("db", 5433, false, &[]),
    ]);
    let written = mutations(&fake);
    // The update of the producer is made, and the object is not ready when it is read again.
    let after = db.clone();
    // Calls 1 to 9 plan the run and read the producer, 10 updates it, 11 reads it again.
    let mut hooked = Hooked::new(&mut fake, 11, move |f| {
        f.set_status(&after, ucdk_provider::Status::Failed);
    });
    let report = done(apply(&mut hooked, &ledger, &changed));
    drop(hooked);
    assert_eq!(report.updated, [path("db/run")], "{report:?}");
    let held = records(&ledger);
    assert_eq!(held[&path("db/run")].state, State::Intent);
    assert_eq!(held[&path("app/run")].state, State::Confirmed);
    assert_eq!(mutations(&fake), written + 1, "only the producer was written");
    assert!(
        objects(&fake).contains("port=80") && !objects(&fake).contains("port=99"),
        "the consumer was left as it was"
    );
}
