// SPDX-License-Identifier: Apache-2.0
//! The fault hooks: each is constructor data, fires as documented, and fires once.

mod common;

use common::{cell, job, node, service, stamp};
use ucdk_provider::{
    Kind, Observation, OwnerScope, PhysicalId, Provider, ProviderError, Refusal, StampEvidence,
    Status,
};
use ucdk_provider_fake::{Fake, FakeConfig, Faults, ForeignObject, Op};

fn with(faults: Faults) -> Fake {
    Fake::new(FakeConfig {
        faults,
        ..FakeConfig::standard()
    })
}

fn present(fake: &mut Fake, node: &ucdk_provider::Node) -> bool {
    matches!(fake.observe(node, None), Ok(Observation::Present(_)))
}

#[test]
fn fake_fail_at_call_refuses_kth_mutation_once() {
    let mut fake = with(Faults {
        fail_at_call: Some(2),
        ..Faults::default()
    });
    let (a, b) = (service("app/a", "a"), service("app/b", "b"));
    assert!(fake.create(&a, &stamp(&a)).is_ok());
    // A read between mutations is not counted.
    assert!(present(&mut fake, &a));
    assert_eq!(
        fake.create(&b, &stamp(&b)),
        Err(ProviderError::Refused(Refusal::Rejected))
    );
    assert!(
        !present(&mut fake, &b),
        "the refused mutation changed nothing"
    );
    assert!(
        fake.create(&b, &stamp(&b)).is_ok(),
        "the next call proceeds"
    );
    assert!(present(&mut fake, &b));
}

#[test]
fn fail_at_call_counts_updates_and_deletes_too() {
    for (k, update_refused, delete_refused) in [(2, true, false), (3, false, true)] {
        let mut fake = with(Faults {
            fail_at_call: Some(k),
            ..Faults::default()
        });
        let a = service("app/a", "a");
        let created = fake.create(&a, &stamp(&a)).expect("create");
        let Ok(Observation::Present(live)) = fake.observe(&a, None) else {
            panic!("created object not found");
        };
        let update = fake.update(&a, &live);
        let delete = fake.delete(a.kind(), &created.physical);
        assert_eq!(update.is_err(), update_refused, "k={k}");
        assert_eq!(delete.is_err(), delete_refused, "k={k}");
    }
}

#[test]
fn fake_read_lag_hides_new_node_until_n_reads() {
    let mut fake = with(Faults {
        read_lag: 2,
        ..Faults::default()
    });
    let a = service("app/a", "a");
    let scope = OwnerScope::new(cell(), a.path().owner());
    let created = fake.create(&a, &stamp(&a)).expect("create");
    assert!(!present(&mut fake, &a), "read 1 is hidden");
    assert_eq!(fake.list_owned(&scope), Ok(vec![]), "read 2 is hidden");
    let Ok(Observation::Present(live)) = fake.observe(&a, None) else {
        panic!("read 3 must show the object");
    };
    assert_eq!(live.physical, created.physical);
    assert_eq!(fake.list_owned(&scope).expect("list").len(), 1);
    // The create itself saw the object: a second create is refused even while it is hidden.
    let mut lagged = with(Faults {
        read_lag: 5,
        ..Faults::default()
    });
    lagged.create(&a, &stamp(&a)).expect("create");
    assert_eq!(
        lagged.create(&a, &stamp(&a)),
        Err(ProviderError::Refused(Refusal::AlreadyExists))
    );
}

#[test]
fn fake_ambiguous_success_mutates_but_times_out() {
    let mut fake = with(Faults {
        ambiguous_success: true,
        ..Faults::default()
    });
    let a = service("app/a", "a");
    let reply = fake.create(&a, &stamp(&a));
    assert!(matches!(reply, Err(ProviderError::Unknown(_))), "{reply:?}");
    assert!(present(&mut fake, &a), "the create happened");
    let logged = &fake.calls()[0];
    assert_eq!((logged.op, logged.outcome.clone()), (Op::Create, Ok(())));
    // It fires once: the next mutation replies normally.
    let b = service("app/b", "b");
    assert!(fake.create(&b, &stamp(&b)).is_ok());
}

#[test]
fn ambiguous_success_applies_to_update_and_delete() {
    let a = service("app/a", "a");
    let mut fake = with(Faults::default());
    let created = fake.create(&a, &stamp(&a)).expect("create");
    let Ok(Observation::Present(live)) = fake.observe(&a, None) else {
        panic!("not found");
    };
    let changed = common::node("app/a", Kind::service(), "a", 81);
    *fake.faults_mut().ambiguous_success = true;
    assert!(matches!(
        fake.update(&changed, &live),
        Err(ProviderError::Unknown(_))
    ));
    let Ok(Observation::Present(now)) = fake.observe(&a, None) else {
        panic!("not found");
    };
    assert_eq!(now.digest, changed.digest(), "the update happened");
    *fake.faults_mut().ambiguous_success = true;
    assert!(matches!(
        fake.delete(a.kind(), &created.physical),
        Err(ProviderError::Unknown(_))
    ));
    assert!(!present(&mut fake, &a), "the delete happened");
}

#[test]
fn fake_race_next_makes_second_create_lose() {
    let mut fake = with(Faults {
        race_next: true,
        ..Faults::default()
    });
    let a = service("app/a", "a");
    assert_eq!(
        fake.create(&a, &stamp(&a)),
        Err(ProviderError::Refused(Refusal::AlreadyExists))
    );
    assert_eq!(fake.object_count(), 1, "exactly one object exists, not two");
    let Ok(Observation::Present(live)) = fake.observe(&a, None) else {
        panic!("the winner's object must be there");
    };
    assert_eq!(
        live.stamp,
        StampEvidence::Present(stamp(&a)),
        "the winner wrote the same stamp, so the loser can adopt it"
    );
    let creates: Vec<_> = fake.calls().iter().filter(|c| c.op == Op::Create).collect();
    assert_eq!(creates.len(), 2);
    assert!(creates[0].outcome.is_ok() && creates[1].outcome.is_err());
    // It fires once.
    let b = service("app/b", "b");
    assert!(fake.create(&b, &stamp(&b)).is_ok());
}

#[test]
fn foreign_objects_are_there_from_the_start_and_never_ours() {
    let mut fake = with(Faults {
        foreign: vec![
            ForeignObject {
                kind: Kind::service(),
                name: "taken".to_owned(),
                marker: Some("other-tool/managed".to_owned()),
            },
            ForeignObject {
                kind: Kind::job(),
                name: "bare".to_owned(),
                marker: None,
            },
        ],
        ..Faults::default()
    });
    let taken = service("app/a", "taken");
    let bare = job("app/b", "bare");
    let Ok(Observation::Present(live)) = fake.observe(&taken, None) else {
        panic!("foreign object missing");
    };
    assert_eq!(
        live.stamp,
        StampEvidence::OtherTool {
            marker: "other-tool/managed".to_owned()
        }
    );
    let Ok(Observation::Present(live)) = fake.observe(&bare, None) else {
        panic!("foreign object missing");
    };
    assert_eq!(live.stamp, StampEvidence::Absent);
    assert_eq!(live.unmanaged, ["origin"]);
    assert_eq!(
        fake.create(&taken, &stamp(&taken)),
        Err(ProviderError::Refused(Refusal::AlreadyExists))
    );
    assert!(fake.calls().iter().all(|c| c.op != Op::Delete));
}

#[test]
fn ambiguous_at_call_times_out_only_the_kth_mutation() {
    let mut fake = with(Faults {
        ambiguous_at_call: Some(2),
        ..Faults::default()
    });
    let (a, b, c) = (
        service("app/a", "a"),
        service("app/b", "b"),
        service("app/c", "c"),
    );
    assert!(fake.create(&a, &stamp(&a)).is_ok());
    assert!(present(&mut fake, &a), "reads are not counted");
    let second = fake.create(&b, &stamp(&b));
    assert!(
        matches!(second, Err(ProviderError::Unknown(_))),
        "{second:?}"
    );
    assert!(present(&mut fake, &b), "the second create happened");
    assert!(fake.create(&c, &stamp(&c)).is_ok(), "the third replies");
}

#[test]
fn ambiguous_at_call_does_not_hide_a_refusal() {
    let mut fake = with(Faults {
        ambiguous_at_call: Some(1),
        ..Faults::default()
    });
    let a = service("app/a", "a");
    fake.add_foreign(&Kind::service(), "a", None);
    assert_eq!(
        fake.create(&a, &stamp(&a)),
        Err(ProviderError::Refused(Refusal::AlreadyExists)),
        "a create that changes nothing is reported as it is"
    );
}

#[test]
fn fail_reads_fails_the_next_reads_in_order_and_changes_nothing() {
    let transient = ProviderError::Transient("busy".to_owned());
    let in_flight = ProviderError::InFlight("still creating".to_owned());
    let mut fake = with(Faults {
        fail_reads: vec![transient.clone(), in_flight.clone()],
        ..Faults::default()
    });
    let a = service("app/a", "a");
    let scope = OwnerScope::new(cell(), a.path().owner());
    fake.create(&a, &stamp(&a)).expect("create");
    assert_eq!(fake.observe(&a, None), Err(transient.clone()));
    assert_eq!(fake.list_owned(&scope), Err(in_flight.clone()));
    assert!(present(&mut fake, &a), "reads work again, nothing was lost");
    assert_eq!(fake.list_owned(&scope).map(|l| l.len()), Ok(1));
    let failed: Vec<_> = fake
        .calls()
        .iter()
        .filter(|c| c.outcome.is_err())
        .map(|c| (c.op, c.outcome.clone()))
        .collect();
    assert_eq!(
        failed,
        [
            (Op::Observe, Err(transient)),
            (Op::ListOwned, Err(in_flight))
        ]
    );
}

#[test]
fn a_failed_read_counts_toward_read_lag() {
    let mut fake = with(Faults {
        read_lag: 1,
        fail_reads: vec![ProviderError::Transient("busy".to_owned())],
        ..Faults::default()
    });
    let a = service("app/a", "a");
    fake.create(&a, &stamp(&a)).expect("create");
    assert!(fake.observe(&a, None).is_err(), "read 1 fails");
    assert!(present(&mut fake, &a), "read 2 is past the lag of 1");
}

#[test]
fn a_deleted_object_is_still_observed_for_n_reads_under_read_lag() {
    let mut fake = with(Faults::default());
    let a = service("app/a", "a");
    let scope = OwnerScope::new(cell(), a.path().owner());
    let created = fake.create(&a, &stamp(&a)).expect("create");
    assert!(present(&mut fake, &a));
    *fake.faults_mut().read_lag = 3;
    fake.delete(a.kind(), &created.physical).expect("delete");
    assert_eq!(fake.object_count(), 0);
    assert!(
        present(&mut fake, &a),
        "stale read 1 sees the deleted object"
    );
    assert_eq!(
        fake.list_owned(&scope),
        Ok(vec![]),
        "read 2: a list tells the truth"
    );
    let hinted = fake.observe(&a, Some(&created.physical));
    assert!(
        matches!(hinted, Ok(Observation::Present(_))),
        "stale read 3, by hint"
    );
    assert!(!present(&mut fake, &a), "read 4 is past the lag of 3");
}

#[test]
fn a_stale_read_matches_the_deleted_object_by_kind_name_and_hint_only() {
    let mut fake = with(Faults {
        read_lag: 9,
        ..Faults::default()
    });
    let a = service("app/a", "a");
    let created = fake.create(&a, &stamp(&a)).expect("create");
    fake.delete(a.kind(), &created.physical).expect("delete");
    let other_name = service("app/b", "b");
    let other_kind = node("app/a", Kind::job(), "a", 80);
    assert!(!present(&mut fake, &other_name), "another name");
    assert!(!present(&mut fake, &other_kind), "another kind");
    let elsewhere = PhysicalId::new("fake-000042");
    assert_eq!(
        fake.observe(&a, Some(&elsewhere)),
        Ok(Observation::Absent),
        "another id"
    );
    assert!(present(&mut fake, &a), "the same kind and name");
}

#[test]
fn a_deleted_object_is_gone_at_the_next_read_without_read_lag() {
    let mut fake = with(Faults::default());
    let a = service("app/a", "a");
    let created = fake.create(&a, &stamp(&a)).expect("create");
    assert!(present(&mut fake, &a));
    fake.delete(a.kind(), &created.physical).expect("delete");
    assert!(!present(&mut fake, &a));
}

#[test]
fn a_race_over_a_taken_name_adds_no_second_object() {
    let mut fake = with(Faults {
        race_next: true,
        foreign: vec![ForeignObject {
            kind: Kind::service(),
            name: "taken".to_owned(),
            marker: None,
        }],
        ..Faults::default()
    });
    let taken = service("app/a", "taken");
    assert_eq!(
        fake.create(&taken, &stamp(&taken)),
        Err(ProviderError::Refused(Refusal::AlreadyExists))
    );
    assert_eq!(fake.object_count(), 1, "one object per kind and name");
    let creates: Vec<_> = fake.calls().iter().filter(|c| c.op == Op::Create).collect();
    assert_eq!(creates.len(), 1, "no racing create was logged");
    // The race was spent: a free name is created normally.
    let free = service("app/b", "free");
    assert!(fake.create(&free, &stamp(&free)).is_ok());
    assert_eq!(fake.object_count(), 2);
}

#[test]
fn faults_armed_on_a_built_fake_fire_like_constructor_faults() {
    let mut fake = Fake::standard();
    let a = service("app/a", "a");
    *fake.faults_mut().race_next = true;
    assert_eq!(
        fake.create(&a, &stamp(&a)),
        Err(ProviderError::Refused(Refusal::AlreadyExists))
    );
    *fake.faults_mut().fail_at_call = Some(2);
    let b = service("app/b", "b");
    assert_eq!(
        fake.create(&b, &stamp(&b)),
        Err(ProviderError::Refused(Refusal::Rejected))
    );
}

#[test]
fn create_failed_fault_births_failed_objects_until_an_update() {
    let mut fake = with(Faults {
        create_failed: true,
        ..Faults::default()
    });
    let a = service("app/a", "a");
    fake.create(&a, &stamp(&a)).expect("create");
    let Ok(Observation::Present(live)) = fake.observe(&a, None) else {
        panic!("not found");
    };
    assert_eq!(live.status, Status::Failed);
    fake.update(&a, &live).expect("update");
    let Ok(Observation::Present(now)) = fake.observe(&a, None) else {
        panic!("not found");
    };
    assert_eq!(now.status, Status::Ready);
    // Disarmed on the built fake, the next object is born ready.
    *fake.faults_mut().create_failed = false;
    let b = service("app/b", "b");
    fake.create(&b, &stamp(&b)).expect("create");
    assert!(matches!(
        fake.observe(&b, None),
        Ok(Observation::Present(l)) if l.status == Status::Ready
    ));
}

#[test]
fn converging_reads_fault_settles_after_that_many_reads() {
    let mut fake = Fake::standard();
    *fake.faults_mut().converging_reads = 2;
    let a = service("app/a", "a");
    fake.create(&a, &stamp(&a)).expect("create");
    let status = |fake: &mut Fake| match fake.observe(&a, None) {
        Ok(Observation::Present(l)) => l.status,
        other => panic!("expected the object, got {other:?}"),
    };
    assert_eq!(status(&mut fake), Status::Converging, "read 1");
    assert_eq!(status(&mut fake), Status::Converging, "read 2");
    assert_eq!(status(&mut fake), Status::Ready, "read 3");
    assert_eq!(status(&mut fake), Status::Ready, "read 4");
}

#[test]
fn converging_counts_from_the_first_visible_read_when_reads_lag() {
    let mut fake = with(Faults {
        read_lag: 1,
        converging_reads: 1,
        ..Faults::default()
    });
    let a = service("app/a", "a");
    fake.create(&a, &stamp(&a)).expect("create");
    assert_eq!(fake.observe(&a, None), Ok(Observation::Absent), "hidden");
    let Ok(Observation::Present(first)) = fake.observe(&a, None) else {
        panic!("visible by the second read");
    };
    assert_eq!(first.status, Status::Converging);
    let Ok(Observation::Present(next)) = fake.observe(&a, None) else {
        panic!("visible");
    };
    assert_eq!(next.status, Status::Ready);
}
