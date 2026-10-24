// SPDX-License-Identifier: Apache-2.0
//! The fault hooks: each is constructor data, fires as documented, and fires once.

mod common;

use common::{job, service, stamp};
use ucdk_provider::{
    Kind, Observation, OwnerScope, Provider, ProviderError, Refusal, StampEvidence,
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
    let scope = OwnerScope::new(a.path().owner());
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
    fake.faults_mut().ambiguous_success = true;
    assert!(matches!(
        fake.update(&changed, &live),
        Err(ProviderError::Unknown(_))
    ));
    let Ok(Observation::Present(now)) = fake.observe(&a, None) else {
        panic!("not found");
    };
    assert_eq!(now.digest, changed.digest(), "the update happened");
    fake.faults_mut().ambiguous_success = true;
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
