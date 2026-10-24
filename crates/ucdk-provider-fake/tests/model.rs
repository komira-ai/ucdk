// SPDX-License-Identifier: Apache-2.0
//! The fake's model: digests, the limited variant, stamps written and read back, and the
//! checks that hold a live object to its kind.

mod common;

use common::{cell, job, node, service, stamp};
use ucdk_provider::kit;
use ucdk_provider::{
    CellScope, Deletability, DesiredField, FieldValue, Kind, Live, Node, NodePath, Observation,
    OwnerScope, PhysicalId, Provider, ProviderError, Refusal, Stamp, StampEvidence, StampMode,
    StateClass, Status,
};
use ucdk_provider_fake::{Fake, FakeConfig, Op};

fn observed(fake: &mut Fake, node: &Node) -> Live {
    match fake.observe(node, None) {
        Ok(Observation::Present(live)) => live,
        other => panic!("expected the object, got {other:?}"),
    }
}

fn digest_with(field: &str, value: &FieldValue) -> String {
    let fields = vec![
        DesiredField::new("port", FieldValue::Int(80)),
        DesiredField::new("label", FieldValue::Text("a".to_owned())),
        DesiredField::new("public", FieldValue::Bool(false)),
    ]
    .into_iter()
    .map(|f| {
        if f.name == field {
            DesiredField::new(field, value.clone())
        } else {
            f
        }
    })
    .collect();
    Node::new(
        NodePath::new("app/a").unwrap(),
        Kind::service(),
        "a",
        fields,
    )
    .unwrap()
    .digest()
    .to_string()
}

#[test]
fn fake_digest_covers_every_modelled_field() {
    let base = service("app/a", "a");
    let mut fake = Fake::standard();
    fake.create(&base, &stamp(&base)).unwrap();
    let live = observed(&mut fake, &base);
    assert_eq!(
        live.digest,
        base.digest(),
        "the fake reads back the node's digest"
    );

    // Changing any one modelled field changes the digest.
    let reference = base.digest().to_string();
    for (field, value) in [
        ("port", FieldValue::Int(81)),
        ("label", FieldValue::Text("b".to_owned())),
        ("public", FieldValue::Bool(true)),
    ] {
        assert_ne!(digest_with(field, &value), reference, "field {field}");
    }
    // The kind is covered too.
    let as_job = node("app/a", Kind::job(), "a", 80);
    assert_ne!(as_job.digest(), base.digest());
    // A value of another type is a different value, even when it prints the same.
    assert_ne!(
        digest_with("port", &FieldValue::Text("80".to_owned())),
        reference
    );

    // What the catalog does not model, and the physical name, are not in it.
    let id = live.physical.clone();
    assert!(fake.set_unmodelled(&id, "note", "hand edited"));
    let after = observed(&mut fake, &base);
    assert_eq!(after.digest, base.digest());
    assert_eq!(after.unmanaged, ["note"]);
    assert_eq!(
        node("app/a", Kind::service(), "renamed", 80).digest(),
        base.digest()
    );
    assert!(!fake.set_unmodelled(&PhysicalId::new("nope"), "k", "v"));
}

#[test]
fn fake_limited_refuses_job_before_creating_anything() {
    let mut fake = Fake::new(FakeConfig::limited());
    let j = job("app/nightly", "nightly");
    assert_eq!(
        fake.create(&j, &stamp(&j)),
        Err(ProviderError::Refused(Refusal::Unsupported))
    );
    assert_eq!(fake.object_count(), 0, "nothing was created");
    assert_eq!(fake.observe(&j, None), Ok(Observation::Absent));
    let creates: Vec<_> = fake.calls().iter().filter(|c| c.op == Op::Create).collect();
    assert_eq!(creates.len(), 1);
    assert!(creates[0].physical.is_none());
    // The kinds it does host work.
    let s = service("app/web", "web");
    assert!(fake.create(&s, &stamp(&s)).is_ok());
}

#[test]
fn fake_golden_state_dump() {
    let mut fake = Fake::standard();
    let web = service("app/web", "web");
    let nightly = job("app/nightly", "nightly");
    fake.create(&web, &stamp(&web)).unwrap();
    fake.create(&nightly, &Stamp::for_node(&cell(), nightly.path()))
        .unwrap();
    let held = fake.add_foreign(&Kind::service(), "legacy", Some("other-tool".to_owned()));
    fake.set_unmodelled(&held, "owner", "ops");
    assert!(fake.create(&web, &stamp(&web)).is_err());
    let live = observed(&mut fake, &web);
    fake.delete(web.kind(), &live.physical).unwrap();
    assert_eq!(fake.dump(), include_str!("golden/state_dump.txt"));
}

#[test]
fn kinds_report_their_state_class_stamp_mode_and_deletability() {
    let stateful = Kind::new("store");
    let mut fake = Fake::new(FakeConfig {
        stateful: [stateful.clone()].into(),
        deletion_forbidden: [stateful.clone()].into(),
        ..FakeConfig::standard()
    });
    assert_eq!(fake.state_class(&stateful), StateClass::Stateful);
    assert_eq!(fake.state_class(&Kind::service()), StateClass::Stateless);
    assert_eq!(fake.stamp_mode(&Kind::service()), StampMode::Tag);
    assert_eq!(fake.stamp_mode(&Kind::job()), StampMode::NameEncoded);

    let store = node("app/data", stateful.clone(), "data", 1);
    let created = fake.create(&store, &stamp(&store)).unwrap();
    let empty = observed(&mut fake, &store);
    assert_eq!(
        fake.deletability(&stateful, &empty),
        Deletability::Deletable
    );
    // Once it holds something the catalog does not model, deletion is refused and says so.
    fake.set_unmodelled(&created.physical, "objects", "3");
    let full = observed(&mut fake, &store);
    assert!(matches!(
        fake.deletability(&stateful, &full),
        Deletability::Undeletable { .. }
    ));
    assert_eq!(
        fake.delete(&stateful, &created.physical),
        Err(ProviderError::Refused(Refusal::Rejected))
    );
    assert_eq!(fake.object_count(), 1);
    // A kind that is not forbidden stays deletable with the same content.
    let s = service("app/web", "web");
    let made = fake.create(&s, &stamp(&s)).unwrap();
    fake.set_unmodelled(&made.physical, "k", "v");
    let live = observed(&mut fake, &s);
    assert_eq!(fake.deletability(s.kind(), &live), Deletability::Deletable);
}

#[test]
fn a_tag_is_written_as_labels_and_read_back_with_its_extra_labels() {
    let mut fake = Fake::standard();
    let s = service("app/web", "web");
    let wanted = stamp(&s).with_extra("run", "r7");
    let created = fake.create(&s, &wanted).unwrap();
    assert_eq!(fake.birth_stamp(&created.physical), Some(wanted.clone()));
    assert_eq!(
        observed(&mut fake, &s).stamp,
        StampEvidence::Present(wanted.clone())
    );
    let owned = fake
        .list_owned(&OwnerScope::new(cell(), s.path().owner()))
        .unwrap();
    assert_eq!(owned.len(), 1);
    assert_eq!(owned[0].stamp, wanted);
    assert_eq!(owned[0].name, "web");
    assert_eq!(fake.birth_stamp(&PhysicalId::new("fake-missing")), None);
}

#[test]
fn a_partial_or_malformed_tag_reads_as_no_stamp() {
    let s = service("app/web", "web");
    for (label, value) in [
        ("ucdk-owner", None),
        ("ucdk-node", None),
        ("ucdk-scheme", None),
        ("ucdk-owner", Some("Not Valid")),
        ("ucdk-node", Some("short")),
        ("ucdk-scheme", Some("one")),
    ] {
        let mut fake = Fake::standard();
        let created = fake.create(&s, &stamp(&s)).unwrap();
        assert!(fake.set_label(&created.physical, label, value));
        assert_eq!(
            observed(&mut fake, &s).stamp,
            StampEvidence::Absent,
            "{label} {value:?}"
        );
        let scope = OwnerScope::new(cell(), s.path().owner());
        assert!(fake.list_owned(&scope).unwrap().is_empty());
    }
    assert!(!Fake::standard().set_label(&PhysicalId::new("nope"), "k", None));
}

#[test]
fn a_tag_less_kind_carries_its_stamp_in_the_name_and_keeps_it_through_a_rename() {
    let mut fake = Fake::standard();
    let j = job("app/nightly", "nightly");
    let created = fake.create(&j, &stamp(&j)).unwrap();
    let live = observed(&mut fake, &j);
    assert_eq!(
        live.name, "nightly",
        "the human name is without the encoding"
    );
    assert_eq!(live.stamp, StampEvidence::Present(stamp(&j)));
    assert_eq!(fake.birth_stamp(&created.physical), Some(stamp(&j)));
    assert!(
        fake.dump()
            .contains("name=nightly~ucdk1~kit-machine~kit-cell~aaaaaaaaaaaaaaaaaaaaaaaaaa~app~"),
        "{}",
        fake.dump()
    );
    assert!(fake.rename(&created.physical, "later"));
    let renamed = fake.observe(&j, Some(&created.physical)).unwrap();
    let Observation::Present(renamed) = renamed else {
        panic!("not found")
    };
    assert_eq!(renamed.name, "later");
    assert_eq!(renamed.stamp, StampEvidence::Present(stamp(&j)));
    assert!(
        fake.dump()
            .contains("name=later~ucdk1~kit-machine~kit-cell~aaaaaaaaaaaaaaaaaaaaaaaaaa~app~")
    );
    // An unstamped object is renamed plainly.
    let bare = fake.add_foreign(&Kind::job(), "bare", None);
    assert!(fake.rename(&bare, "bare2"));
    assert!(fake.dump().contains("name=bare2 "));
    assert!(!fake.rename(&PhysicalId::new("nope"), "x"));
}

#[test]
fn update_and_delete_check_the_object_and_its_kind() {
    let mut fake = Fake::standard();
    let s = service("app/web", "web");
    let created = fake.create(&s, &stamp(&s)).unwrap();
    let live = observed(&mut fake, &s);
    let ghost = Live {
        physical: PhysicalId::new("fake-999999"),
        ..live.clone()
    };
    assert_eq!(
        fake.update(&s, &ghost),
        Err(ProviderError::Refused(Refusal::NotFound))
    );
    let as_job = node("app/web", Kind::job(), "web", 80);
    assert!(matches!(
        fake.update(&as_job, &live),
        Err(ProviderError::Permanent(_))
    ));
    assert!(matches!(
        fake.delete(&Kind::job(), &created.physical),
        Err(ProviderError::Permanent(_))
    ));
    assert_eq!(fake.object_count(), 1, "neither changed anything");
    assert!(
        fake.dump()
            .contains("update job fake-000001 permanent: a change of kind")
    );
}

#[test]
fn observe_by_hint_finds_the_object_only_if_it_is_there_and_of_the_nodes_kind() {
    let mut fake = Fake::standard();
    let s = service("app/web", "web");
    let created = fake.create(&s, &stamp(&s)).unwrap();
    let as_job = node("app/web", Kind::job(), "web", 80);
    assert_eq!(
        fake.observe(&as_job, Some(&created.physical)),
        Ok(Observation::Absent)
    );
    assert_eq!(
        fake.observe(&s, Some(&PhysicalId::new("fake-999999"))),
        Ok(Observation::Absent)
    );
    assert!(matches!(
        fake.observe(&s, Some(&created.physical)),
        Ok(Observation::Present(_))
    ));
}

#[test]
fn list_owned_lists_only_stamped_objects_of_the_scope_owner() {
    let mut fake = Fake::standard();
    let mine = service("app/web", "web");
    let theirs = service("other/web", "web2");
    fake.create(&mine, &stamp(&mine)).unwrap();
    fake.create(&theirs, &stamp(&theirs)).unwrap();
    fake.add_foreign(&Kind::service(), "legacy", Some("other-tool".to_owned()));
    fake.add_foreign(&Kind::service(), "bare", None);
    let owned = fake
        .list_owned(&OwnerScope::new(cell(), mine.path().owner()))
        .unwrap();
    assert_eq!(owned.len(), 1);
    assert_eq!(owned[0].stamp.owner, mine.path().owner());
}

#[test]
fn a_deletion_forbidden_kind_may_be_deleted_while_it_holds_nothing() {
    let store = Kind::new("store");
    let mut fake = Fake::new(FakeConfig {
        deletion_forbidden: [store.clone()].into(),
        ..FakeConfig::standard()
    });
    let empty = node("app/data", store.clone(), "data", 1);
    let created = fake.create(&empty, &stamp(&empty)).unwrap();
    assert_eq!(fake.delete(&store, &created.physical), Ok(()));
    assert_eq!(fake.object_count(), 0);
}

#[test]
fn every_call_kind_has_a_name_and_says_whether_it_mutates() {
    let mut fake = Fake::standard();
    let s = service("app/web", "web");
    let created = fake.create(&s, &stamp(&s)).unwrap();
    let live = observed(&mut fake, &s);
    fake.update(&s, &live).unwrap();
    fake.list_owned(&OwnerScope::new(cell(), s.path().owner()))
        .unwrap();
    fake.delete(s.kind(), &created.physical).unwrap();
    let seen: Vec<(&str, bool)> = fake
        .calls()
        .iter()
        .map(|c| (c.op.name(), c.op.mutates()))
        .collect();
    assert_eq!(
        seen,
        [
            ("create", true),
            ("observe", false),
            ("update", true),
            ("list_owned", false),
            ("delete", true)
        ]
    );
}

#[test]
fn birth_stamp_is_read_from_the_stored_object_not_the_call_log() {
    let mut fake = Fake::standard();
    let (web, task) = (service("app/web", "web"), job("app/task", "task"));
    let web_id = fake.create(&web, &stamp(&web)).unwrap().physical;
    let task_id = fake.create(&task, &stamp(&task)).unwrap().physical;
    fake.clear_calls();
    assert!(fake.calls().is_empty());
    assert!(
        fake.birth_stamp(&web_id)
            .unwrap()
            .same_identity(&stamp(&web))
    );
    assert!(
        fake.birth_stamp(&task_id)
            .unwrap()
            .same_identity(&stamp(&task))
    );
}

#[test]
fn a_stamp_set_after_birth_is_not_a_birth_stamp_and_a_changed_one_does_not_rewrite_it() {
    let mut fake = Fake::standard();
    let web = service("app/web", "web");
    let born = fake.create(&web, &stamp(&web)).unwrap().physical;
    let s = fake.birth_stamp(&born).unwrap();
    let late = fake.add_foreign(&Kind::service(), "late", None);
    for (key, value) in [
        ("ucdk-machine", s.cell.machine().to_string()),
        ("ucdk-cell", s.cell.cell().to_string()),
        ("ucdk-cell-uid", s.cell.uid().to_string()),
        ("ucdk-owner", s.owner.to_string()),
        ("ucdk-node", s.node.to_string()),
        ("ucdk-scheme", s.scheme.to_string()),
    ] {
        assert!(fake.set_label(&late, key, Some(&value)));
    }
    let named = service("app/late", "late");
    assert!(
        matches!(observed(&mut fake, &named).stamp, StampEvidence::Present(_)),
        "the object now carries a stamp"
    );
    assert_eq!(fake.birth_stamp(&late), None, "but it was born without one");
    // Changing the owner label afterwards does not change who it was born as.
    assert!(fake.set_label(&born, "ucdk-owner", Some("someone-else")));
    assert!(fake.birth_stamp(&born).unwrap().same_identity(&stamp(&web)));
    assert_eq!(fake.birth_stamp(&PhysicalId::new("nothing")), None);
}

#[test]
fn a_name_encoded_kind_refuses_extra_labels_and_creates_nothing() {
    let mut fake = Fake::standard();
    let nightly = job("app/nightly", "nightly");
    let asked = Stamp::for_node(&cell(), nightly.path()).with_extra("run", "r1");
    assert_eq!(
        fake.create(&nightly, &asked),
        Err(ProviderError::Refused(Refusal::Unsupported))
    );
    assert_eq!(fake.object_count(), 0, "nothing was created");
    assert_eq!(fake.observe(&nightly, None), Ok(Observation::Absent));
    assert_eq!(
        fake.list_owned(&OwnerScope::new(cell(), nightly.path().owner())),
        Ok(Vec::new())
    );
    // Without extra labels the same kind is created as before.
    assert!(fake.create(&nightly, &stamp(&nightly)).is_ok());
}

#[test]
fn stamp_modes_say_whether_extra_labels_are_carried() {
    let fake = Fake::standard();
    for kind in [Kind::service(), Kind::job()] {
        assert_eq!(
            fake.stamp_mode(&kind).carries_extra(),
            kind == Kind::service(),
            "only the tagged kind carries extra labels"
        );
    }
}

#[test]
fn observe_reports_failed_status() {
    let mut fake = Fake::standard();
    let s = service("app/web", "web");
    let created = fake.create(&s, &stamp(&s)).unwrap();
    assert_eq!(observed(&mut fake, &s).status, Status::Ready);
    assert!(fake.set_status(&created.physical, Status::Failed));
    let failed = observed(&mut fake, &s);
    assert_eq!(failed.status, Status::Failed);
    assert_eq!(
        failed.digest,
        s.digest(),
        "the fields still match: only the status says the object is failed"
    );
    let by_id = fake.observe(&s, Some(&created.physical));
    assert!(matches!(by_id, Ok(Observation::Present(l)) if l.status == Status::Failed));
    // An update converges the object.
    fake.update(&s, &failed).unwrap();
    assert_eq!(observed(&mut fake, &s).status, Status::Ready);
    assert!(!fake.set_status(&PhysicalId::new("nope"), Status::Failed));
}

#[test]
fn observe_reports_converging_status_until_an_update() {
    let mut fake = Fake::standard();
    let s = service("app/web", "web");
    let created = fake.create(&s, &stamp(&s)).unwrap();
    assert!(fake.set_status(&created.physical, Status::Converging));
    for _ in 0..3 {
        assert_eq!(observed(&mut fake, &s).status, Status::Converging);
    }
    let live = observed(&mut fake, &s);
    fake.update(&s, &live).unwrap();
    assert_eq!(observed(&mut fake, &s).status, Status::Ready);
}

#[test]
fn list_stamped_returns_all_owners_skips_foreign() {
    let mut fake = Fake::standard();
    let web = service("app/web", "web");
    let other = service("other/web", "web2");
    let nightly = job("lib/nightly", "nightly");
    let mut ids = Vec::new();
    for n in [&web, &other, &nightly] {
        ids.push(fake.create(n, &stamp(n)).unwrap().physical);
    }
    fake.add_foreign(&Kind::service(), "legacy", Some("other-tool".to_owned()));
    fake.add_foreign(&Kind::service(), "bare", None);
    let listed = fake
        .list_stamped(&CellScope::new(cell()))
        .expect("list_stamped");
    let found: Vec<&PhysicalId> = listed.iter().map(|o| &o.physical).collect();
    assert_eq!(found, ids.iter().collect::<Vec<_>>());
    let owners: Vec<&str> = listed.iter().map(|o| o.stamp.owner.as_str()).collect();
    assert_eq!(owners, ["app", "other", "lib"]);
    let last = fake.calls().last().expect("the call is logged");
    assert_eq!(last.op, Op::ListStamped);
    assert_eq!(last.op.name(), "list_stamped");
    assert!(!last.op.mutates());
}

#[test]
fn list_stamped_ignores_other_cell() {
    let mut fake = Fake::standard();
    let mine = service("app/web", "web");
    let theirs = service("app/web2", "web2");
    let theirs_tagless = job("app/nightly", "nightly");
    let mine_id = fake.create(&mine, &stamp(&mine)).unwrap().physical;
    let other = kit::other_cell();
    let theirs_id = fake
        .create(&theirs, &Stamp::for_node(&other, theirs.path()))
        .unwrap()
        .physical;
    let tagless_id = fake
        .create(
            &theirs_tagless,
            &Stamp::for_node(&other, theirs_tagless.path()),
        )
        .unwrap()
        .physical;
    let ids = |fake: &mut Fake, cell| {
        let mut stamped: Vec<PhysicalId> = fake
            .list_stamped(&CellScope::new(cell))
            .unwrap()
            .into_iter()
            .map(|o| o.physical)
            .collect();
        stamped.sort();
        stamped
    };
    assert_eq!(ids(&mut fake, cell()), [mine_id]);
    assert_eq!(
        ids(&mut fake, other.clone()),
        [theirs_id.clone(), tagless_id]
    );
    // list_owned is scoped to the cell as well, though the owner is the same.
    let scope = OwnerScope::new(other, mine.path().owner());
    let owned: Vec<PhysicalId> = fake
        .list_owned(&scope)
        .unwrap()
        .into_iter()
        .map(|o| o.physical)
        .collect();
    assert!(owned.contains(&theirs_id));
    assert_eq!(owned.len(), 2);
}

#[test]
fn list_stamped_skips_an_object_of_another_scheme() {
    let mut fake = Fake::standard();
    let s = service("app/web", "web");
    let created = fake.create(&s, &stamp(&s)).unwrap();
    assert_eq!(fake.list_stamped(&CellScope::new(cell())).unwrap().len(), 1);
    assert!(fake.set_label(&created.physical, "ucdk-scheme", Some("2")));
    assert!(
        fake.list_stamped(&CellScope::new(cell()))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn a_tag_with_a_missing_or_malformed_cell_label_is_not_a_stamp() {
    let s = service("app/web", "web");
    for (label, bad) in [
        ("ucdk-machine", Some("Machine")),
        ("ucdk-cell", Some("Cell")),
        ("ucdk-cell-uid", Some("short")),
        ("ucdk-machine", None),
        ("ucdk-cell", None),
        ("ucdk-cell-uid", None),
    ] {
        let mut fake = Fake::standard();
        let created = fake.create(&s, &stamp(&s)).unwrap();
        assert!(fake.set_label(&created.physical, label, bad));
        assert_eq!(
            observed(&mut fake, &s).stamp,
            StampEvidence::Absent,
            "{label}"
        );
    }
}

#[test]
fn an_embedded_kind_is_declared_and_refused() {
    let mut fake = Fake::new(FakeConfig::with_embedded());
    let grant = node("app/grant", Kind::new("grant"), "grant", 1);
    assert_eq!(fake.stamp_mode(grant.kind()), StampMode::Embedded);
    assert_eq!(fake.stamp_mode(&Kind::service()), StampMode::Tag);
    assert_eq!(
        fake.create(&grant, &stamp(&grant)),
        Err(ProviderError::Refused(Refusal::Unsupported))
    );
    assert_eq!(fake.object_count(), 0);
    assert!(
        fake.hosts(grant.kind()),
        "it is hosted, and not yet applied"
    );
}

#[test]
fn golden_state_dump_includes_status() {
    let mut fake = Fake::standard();
    let kinds = [("app/a", "a"), ("app/b", "b"), ("app/c", "c")];
    let mut ids = Vec::new();
    for (path, name) in kinds {
        let s = service(path, name);
        ids.push(fake.create(&s, &stamp(&s)).unwrap().physical);
    }
    assert!(fake.set_status(&ids[1], Status::Converging));
    assert!(fake.set_status(&ids[2], Status::Failed));
    assert_eq!(fake.dump(), include_str!("golden/state_dump_status.txt"));
}
