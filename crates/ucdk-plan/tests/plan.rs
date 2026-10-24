// SPDX-License-Identifier: Apache-2.0
//! The plan: what it does with an observed state, what it refuses, and its hash.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_plan::{
    Action, Conflict, ConflictKind, LedgerToken, LiveState, Lowered, Observed, Plan, PlanNode,
    Record, Refused, Retained, Seen, Step, lower, plan,
};
use ucdk_proto::v1::r#ref::Output as Arm;
use ucdk_proto::v1::resource::Body;
use ucdk_proto::v1::{
    Access, Image, Output, Ref, Resource, ResourceList, Service, Uses, Value, image, service, value,
};
use ucdk_provider::{
    Deletability, Kind, Live, NodePath, PhysicalId, Stamp, StampEvidence, StateClass,
};

// ---- builders --------------------------------------------------------------------------------

fn service(id: &str, port: u32) -> Resource {
    Resource {
        id: id.to_owned(),
        body: Some(Body::Service(Service {
            image: Some(Image {
                source: Some(image::Source::Digest("sha256:aa".to_owned())),
                platform: String::new(),
            }),
            port,
            ..Service::default()
        })),
        ..Resource::default()
    }
}

/// `app` calls `db`.
fn app_and_db(port: u32) -> Lowered {
    let mut app = service("app", port);
    app.uses.push(Uses {
        target: Some(Ref {
            resource: "db".to_owned(),
            output: None,
        }),
        access: Access::Call as i32,
    });
    lower(&ResourceList {
        resource: vec![app, service("db", 5432)],
    })
    .expect("the list lowers")
}

fn path(text: &str) -> NodePath {
    NodePath::new(text).expect("a valid path")
}

fn node<'a>(l: &'a Lowered, p: &str) -> &'a PlanNode {
    l.nodes()
        .iter()
        .find(|n| n.node.path().as_str() == p)
        .expect("the node is there")
}

fn physical(p: &str) -> PhysicalId {
    PhysicalId::new(format!("id-{p}"))
}

fn observed() -> Observed {
    Observed::new(LedgerToken::new("v1"))
}

/// An object that is ours, matches `n`, and is recorded.
fn ours(n: &PlanNode) -> Seen {
    let p = n.node.path();
    Seen {
        path: p.clone(),
        kind: n.node.kind().clone(),
        state_class: StateClass::Stateless,
        live: Some(LiveState {
            live: Live {
                physical: physical(p.as_str()),
                name: n.node.name().to_owned(),
                stamp: StampEvidence::Present(Stamp::for_node(p)),
                digest: n.node.digest(),
                unmanaged: Vec::new(),
            },
            deletability: Deletability::Deletable,
            failed: false,
        }),
        record: Some(Record {
            physical: Some(physical(p.as_str())),
            confirmed: true,
            deps: n.deps.clone(),
        }),
    }
}

/// An object at a path the file does not (or no longer) wants: ours, recorded, with the
/// recorded edges given.
fn leftover(p: &str, deps: &[&str]) -> Seen {
    Seen {
        path: path(p),
        kind: Kind::new("service"),
        state_class: StateClass::Stateless,
        live: Some(LiveState {
            live: Live {
                physical: physical(p),
                name: p.replace('/', "-"),
                stamp: StampEvidence::Present(Stamp::for_node(&path(p))),
                digest: node_digest(),
                unmanaged: Vec::new(),
            },
            deletability: Deletability::Deletable,
            failed: false,
        }),
        record: Some(Record {
            physical: Some(physical(p)),
            confirmed: true,
            deps: deps.iter().map(|d| path(d)).collect(),
        }),
    }
}

fn node_digest() -> ucdk_provider::Digest {
    node(&app_and_db(1), "db/run").node.digest()
}

fn live_mut(seen: &mut Seen) -> &mut LiveState {
    seen.live.as_mut().expect("the object is there")
}

fn steps(p: &Plan) -> Vec<(&str, &Action)> {
    p.steps()
        .iter()
        .map(|s| (s.path.as_str(), &s.action))
        .collect()
}

fn run(l: &Lowered, o: &Observed) -> Plan {
    plan(l, o, None).expect("the run is not refused")
}

fn refused(l: &Lowered, o: &Observed) -> Vec<(String, ConflictKind)> {
    let Refused { conflicts } = plan(l, o, None).expect_err("the run is refused");
    conflicts
        .into_iter()
        .map(|c| (c.path.to_string(), c.why))
        .collect()
}

/// `app` takes `db`'s URL as its `DB` setting: a reference, or the literal `ref:db.URL`.
fn app_with_db_url(literal: bool) -> Lowered {
    let mut app = service("app", 80);
    let v = if literal {
        value::V::Literal("ref:db.URL".to_owned())
    } else {
        value::V::Ref(Ref {
            resource: "db".to_owned(),
            output: Some(Arm::Standard(Output::Url as i32)),
        })
    };
    if let Some(Body::Service(s)) = app.body.as_mut() {
        s.env = BTreeMap::from([("DB".to_owned(), Value { v: Some(v) })]);
    }
    lower(&ResourceList {
        resource: vec![app, service("db", 5432)],
    })
    .expect("the list lowers")
}

/// `n` as it stands live when its referenced values are `url`.
fn ours_bound(n: &PlanNode, url: &str) -> Seen {
    let mut seen = ours(n);
    let bound = n.bound(|_, _| Some(url)).expect("the output is given");
    live_mut(&mut seen).live.digest = bound.digest();
    seen
}

// ---- Plan: what it does ----------------------------------------------------------------------

#[test]
fn plan_mutates_nothing() {
    // The signature is the first half of the property: both inputs are borrowed, and the
    // result is a new value.
    let _: fn(&Lowered, &Observed, Option<&str>) -> Result<Plan, Refused> = plan;
    let l = app_and_db(80);
    let o = observed()
        .with(ours(node(&l, "db/run")))
        .with(leftover("app/old", &["app/run"]));
    let (l0, o0) = (l.clone(), o.clone());
    let first = plan(&l, &o, None);
    assert_eq!(l, l0);
    assert_eq!(o, o0);
    assert_eq!(
        plan(&l, &o, None),
        first,
        "the same inputs give the same plan"
    );
}

#[test]
fn plan_orders_creates_forward_and_deletes_reverse() {
    let l = app_and_db(80);
    // Retired roles of `app`, with their recorded edges: sidecar needs proxy.
    let o = observed()
        .with(leftover("app/sidecar", &["app/proxy"]))
        .with(leftover("app/proxy", &["app/run"]))
        .with(leftover("app/public", &["app/run"]));
    let p = run(&l, &o);
    let delete = |name: &str| Action::Delete {
        physical: physical(name),
    };
    assert_eq!(
        steps(&p),
        [
            ("db/run", &Action::Create),
            ("app/run", &Action::Create),
            ("app/sidecar", &delete("app/sidecar")),
            ("app/public", &delete("app/public")),
            ("app/proxy", &delete("app/proxy")),
        ]
    );
}

#[test]
fn a_node_that_exists_is_created_updated_or_left_alone() {
    let l = app_and_db(80);
    let mut changed = ours(node(&l, "db/run"));
    live_mut(&mut changed).live.digest = node_digest_of_other();
    let o = observed().with(changed).with(ours(node(&l, "app/run")));
    let p = run(&l, &o);
    assert_eq!(
        steps(&p),
        [("db/run", &Action::Update), ("app/run", &Action::Noop)]
    );
    assert_eq!(p.steps()[0].kind.as_str(), "service");

    // A failed object is updated even when its fields match.
    let mut failed = ours(node(&l, "db/run"));
    live_mut(&mut failed).failed = true;
    let p = run(&l, &observed().with(failed));
    assert_eq!(steps(&p)[0], ("db/run", &Action::Update));
}

fn node_digest_of_other() -> ucdk_provider::Digest {
    node(&app_and_db(81), "app/run").node.digest()
}

#[test]
fn an_unconfirmed_intent_that_matches_is_ours() {
    let l = app_and_db(80);
    let mut intent = ours(node(&l, "db/run"));
    intent.record = Some(Record {
        physical: None,
        confirmed: false,
        deps: BTreeSet::new(),
    });
    let p = run(&l, &observed().with(intent));
    assert_eq!(steps(&p)[0], ("db/run", &Action::Noop));
}

#[test]
fn an_intent_with_no_object_is_created() {
    let l = app_and_db(80);
    let mut intent = ours(node(&l, "db/run"));
    intent.live = None;
    intent.record = Some(Record {
        physical: None,
        confirmed: false,
        deps: BTreeSet::new(),
    });
    let p = run(&l, &observed().with(intent));
    assert_eq!(steps(&p)[0], ("db/run", &Action::Create));
}

// ---- Plan: removals --------------------------------------------------------------------------

#[test]
fn a_role_turned_off_is_deleted_when_ours() {
    let l = app_and_db(80);
    assert!(!node(&l, "db/public").wanted);
    let o = observed()
        .with(ours(node(&l, "db/public")))
        .with(ours(node(&l, "db/run")))
        .with(ours(node(&l, "app/run")));
    let p = run(&l, &o);
    assert_eq!(
        steps(&p),
        [
            ("db/run", &Action::Noop),
            ("app/run", &Action::Noop),
            (
                "db/public",
                &Action::Delete {
                    physical: physical("db/public")
                }
            ),
        ]
    );
    assert_eq!(p.steps()[2].kind.as_str(), "public");
}

#[test]
fn removed_node_deleted_only_if_stamp_and_record() {
    let l = app_and_db(80);
    // Stamp and record agree: deleted.
    let o = observed().with(ours(node(&l, "db/public")));
    assert!(matches!(steps(&run(&l, &o))[2].1, Action::Delete { .. }));
    // Stamp without a record, or a record without the stamp: the run is refused, nothing is
    // deleted.
    let mut no_record = ours(node(&l, "db/public"));
    no_record.record = None;
    assert_eq!(
        refused(&l, &observed().with(no_record)),
        [("db/public".to_owned(), ConflictKind::StampWithoutRecord)]
    );
    let mut no_stamp = ours(node(&l, "db/public"));
    live_mut(&mut no_stamp).live.stamp = StampEvidence::Absent;
    assert_eq!(
        refused(&l, &observed().with(no_stamp)),
        [("db/public".to_owned(), ConflictKind::StampStripped)]
    );
}

#[test]
fn keep_node_retained_on_removal() {
    let l = app_and_db(80);
    let mut data = ours(node(&l, "db/public"));
    data.state_class = StateClass::Stateful;
    let p = run(&l, &observed().with(data));
    assert_eq!(steps(&p)[2], ("db/public", &Action::Retain(Retained::Keep)));
}

#[test]
fn undeletable_is_retained_with_the_providers_reason() {
    let l = app_and_db(80);
    let mut locked = ours(node(&l, "db/public"));
    live_mut(&mut locked).deletability = Deletability::Undeletable {
        reason: "holds objects".to_owned(),
    };
    let p = run(&l, &observed().with(locked));
    assert_eq!(
        steps(&p)[2].1,
        &Action::Retain(Retained::Undeletable("holds objects".to_owned()))
    );
}

#[test]
fn an_owner_the_file_no_longer_names_is_retained_not_deleted() {
    let l = app_and_db(80);
    let p = run(&l, &observed().with(leftover("gone/run", &[])));
    assert_eq!(
        steps(&p)[2],
        ("gone/run", &Action::Retain(Retained::OwnerNotNamed))
    );
}

#[test]
fn a_recorded_object_that_is_gone_is_reaped_without_a_call() {
    let l = app_and_db(80);
    let mut gone = leftover("app/old", &[]);
    gone.live = None;
    let nothing_at_all = Seen {
        path: path("app/other"),
        kind: Kind::service(),
        state_class: StateClass::Stateless,
        live: None,
        record: None,
    };
    let p = run(&l, &observed().with(gone).with(nothing_at_all));
    assert_eq!(steps(&p)[2..], [("app/old", &Action::Reap)]);
    assert_eq!(p.steps().len(), 3);
}

// ---- Plan: refusals --------------------------------------------------------------------------

#[test]
fn plan_refuses_whole_run_on_foreign_resource() {
    let l = app_and_db(80);
    let mut foreign = ours(node(&l, "app/run"));
    foreign.record = None;
    live_mut(&mut foreign).live.stamp = StampEvidence::Absent;
    // A foreign object on a path the file does not want refuses the run too, and every
    // offender is listed, in path order. The healthy node is not planned.
    let mut stray = ours(node(&l, "db/public"));
    stray.record = None;
    live_mut(&mut stray).live.stamp = StampEvidence::Absent;
    let o = observed()
        .with(foreign)
        .with(stray)
        .with(ours(node(&l, "db/run")));
    assert_eq!(
        refused(&l, &o),
        [
            ("app/run".to_owned(), ConflictKind::Foreign),
            ("db/public".to_owned(), ConflictKind::Foreign),
        ]
    );
}

#[test]
fn each_way_an_object_is_not_ours_refuses_the_run() {
    let l = app_and_db(80);
    let base = || ours(node(&l, "db/run"));
    let one = |seen: Seen| refused(&l, &observed().with(seen)).remove(0).1;

    let mut s = base();
    s.record = Some(Record {
        physical: Some(PhysicalId::new("another")),
        confirmed: true,
        deps: BTreeSet::new(),
    });
    assert_eq!(one(s), ConflictKind::PhysicalIdMismatch);

    let mut s = base();
    live_mut(&mut s).live.stamp = StampEvidence::Present(Stamp::for_node(&path("app/run")));
    assert_eq!(one(s), ConflictKind::OtherOwner);

    let mut s = base();
    let mut stamp = Stamp::for_node(&path("db/run"));
    stamp.scheme = 2;
    live_mut(&mut s).live.stamp = StampEvidence::Present(stamp);
    assert_eq!(one(s), ConflictKind::OtherOwner);

    let mut s = base();
    live_mut(&mut s).live.stamp = StampEvidence::OtherTool {
        marker: "tool-x".to_owned(),
    };
    assert_eq!(one(s), ConflictKind::OtherTool("tool-x".to_owned()));
}

#[test]
fn an_extra_label_on_the_stamp_does_not_make_an_object_foreign() {
    let l = app_and_db(80);
    let mut s = ours(node(&l, "db/run"));
    live_mut(&mut s).live.stamp =
        StampEvidence::Present(Stamp::for_node(&path("db/run")).with_extra("run", "7"));
    assert_eq!(steps(&run(&l, &observed().with(s)))[0].1, &Action::Noop);
}

#[test]
fn a_kept_object_that_vanished_is_not_recreated_unasked() {
    let l = app_and_db(80);
    let gone = |class, confirmed| {
        let mut s = ours(node(&l, "db/run"));
        s.live = None;
        s.state_class = class;
        s.record.as_mut().expect("a record").confirmed = confirmed;
        s
    };
    assert_eq!(
        refused(&l, &observed().with(gone(StateClass::Stateful, true))),
        [("db/run".to_owned(), ConflictKind::VanishedRetained)]
    );
    // A stateless object is recreated; so is a stateful one whose create never confirmed.
    for s in [
        gone(StateClass::Stateless, true),
        gone(StateClass::Stateful, false),
    ] {
        assert_eq!(steps(&run(&l, &observed().with(s)))[0].1, &Action::Create);
    }
    // A kept object that is gone and whose node is turned off is only reaped.
    let mut off = ours(node(&l, "db/public"));
    off.live = None;
    off.state_class = StateClass::Stateful;
    assert_eq!(steps(&run(&l, &observed().with(off)))[2].1, &Action::Reap);
}

#[test]
fn recorded_edges_that_form_a_cycle_refuse_the_run() {
    let l = app_and_db(80);
    let o = observed()
        .with(leftover("app/a", &["app/b"]))
        .with(leftover("app/b", &["app/a"]))
        .with(leftover("app/c", &[]));
    assert_eq!(
        refused(&l, &o),
        [
            ("app/a".to_owned(), ConflictKind::RecordedCycle),
            ("app/b".to_owned(), ConflictKind::RecordedCycle),
        ]
    );
}

#[test]
fn each_conflict_reads_as_a_sentence_naming_its_path() {
    let all = [
        ConflictKind::Foreign,
        ConflictKind::StampWithoutRecord,
        ConflictKind::PhysicalIdMismatch,
        ConflictKind::StampStripped,
        ConflictKind::OtherOwner,
        ConflictKind::OtherTool("tool-x".to_owned()),
        ConflictKind::VanishedRetained,
        ConflictKind::RecordedCycle,
    ];
    let mut seen = BTreeSet::new();
    for why in all {
        let text = Conflict {
            path: path("web/run"),
            why,
        }
        .to_string();
        assert!(text.starts_with("web/run: "), "{text}");
        assert!(seen.insert(text), "two conflicts read the same");
    }
}

// ---- Plan hash -------------------------------------------------------------------------------

#[test]
fn plan_hash_ignores_ledger_token() {
    let l = app_and_db(80);
    let with_token = |t: &str| {
        Observed::new(LedgerToken::new(t))
            .with(ours(node(&l, "db/run")))
            .with(leftover("app/old", &["app/run"]))
    };
    let (a, b) = (with_token("snapshot-1"), with_token("snapshot-2"));
    assert_ne!(a, b);
    assert_eq!(a.token().as_str(), "snapshot-1");
    let (pa, pb) = (run(&l, &a), run(&l, &b));
    assert_eq!(pa.hash(), pb.hash());
    assert_eq!(pa.hash().to_string(), pa.hash().as_str());
    assert_eq!(pa.hash().as_str().len(), 64);
}

#[test]
fn plan_hash_follows_the_desired_graph() {
    let o = observed();
    let a = run(&app_and_db(80), &o);
    assert_eq!(a.hash(), run(&app_and_db(80), &o).hash());
    // One modelled field differs: the creates are the same steps, the hash is not.
    let b = run(&app_and_db(81), &o);
    assert_eq!(a.steps(), b.steps());
    assert_ne!(a.hash(), b.hash());
}

#[test]
fn plan_hash_has_a_slot_for_the_provider_version() {
    let (l, o) = (app_and_db(80), observed());
    let hash = |v: Option<&str>| plan(&l, &o, v).expect("planned").hash().clone();
    assert_ne!(hash(None), hash(Some("1.0")));
    assert_ne!(hash(Some("1.0")), hash(Some("1.1")));
    assert_eq!(hash(Some("1.0")), hash(Some("1.0")));
}

#[test]
fn plan_hash_covers_every_kind_of_step() {
    let l = app_and_db(80);
    let both = || {
        observed()
            .with(ours(node(&l, "db/run")))
            .with(ours(node(&l, "app/run")))
    };
    let extra = |seen: Seen| run(&l, &both().with(seen)).hash().clone();
    let keep = {
        let mut s = leftover("app/old", &[]);
        s.state_class = StateClass::Stateful;
        s
    };
    let locked = {
        let mut s = leftover("app/old", &[]);
        live_mut(&mut s).deletability = Deletability::Undeletable {
            reason: "r".to_owned(),
        };
        s
    };
    let locked_other_reason = {
        let mut s = locked.clone();
        live_mut(&mut s).deletability = Deletability::Undeletable {
            reason: "other".to_owned(),
        };
        s
    };
    let reaped = {
        let mut s = leftover("app/old", &[]);
        s.live = None;
        s
    };
    let other_object = {
        let mut s = leftover("app/old", &[]);
        live_mut(&mut s).live.physical = PhysicalId::new("different");
        s.record.as_mut().expect("a record").physical = Some(PhysicalId::new("different"));
        s
    };
    let hashes = [
        run(&l, &observed()).hash().clone(),
        run(&l, &both()).hash().clone(),
        run(&l, &observed().with(ours(node(&l, "db/run"))))
            .hash()
            .clone(),
        extra(leftover("app/old", &[])),
        extra(other_object),
        extra(reaped),
        extra(keep),
        extra(locked),
        extra(locked_other_reason),
        extra(leftover("gone/run", &[])),
    ];
    let distinct: BTreeSet<&str> = hashes.iter().map(ucdk_plan::PlanHash::as_str).collect();
    assert_eq!(
        distinct.len(),
        hashes.len(),
        "two different plans share a hash"
    );
    let mut updated = both();
    let mut changed = ours(node(&l, "db/run"));
    live_mut(&mut changed).live.digest = node_digest_of_other();
    updated = updated.with(changed);
    assert!(!hashes.iter().any(|h| *h == *run(&l, &updated).hash()));
}

#[test]
fn a_step_names_its_path_and_kind() {
    let p = run(&app_and_db(80), &observed());
    let first: &Step = &p.steps()[0];
    assert_eq!(first.path, path("db/run"));
    assert_eq!(first.kind, Kind::service());
}

// ---- Plan: references to another resource's output -------------------------------------------

#[test]
fn converged_consumer_of_a_reference_is_a_noop() {
    let l = app_with_db_url(false);
    let o = observed()
        .with_output("db", "URL", "db-url-1")
        .with(ours(node(&l, "db/run")))
        .with(ours_bound(node(&l, "app/run"), "db-url-1"));
    let p = run(&l, &o);
    assert!(
        p.steps().iter().all(|s| s.action == Action::Noop),
        "{:?}",
        steps(&p)
    );
}

#[test]
fn consumer_is_updated_when_the_recorded_output_changed() {
    let l = app_with_db_url(false);
    let o = observed()
        .with_output("db", "URL", "db-url-1")
        .with(ours(node(&l, "db/run")))
        .with(ours_bound(node(&l, "app/run"), "db-url-0"));
    let p = run(&l, &o);
    assert_eq!(
        steps(&p)[..2],
        [("db/run", &Action::Noop), ("app/run", &Action::Update)]
    );
}

#[test]
fn consumer_of_an_output_not_yet_known_is_created_or_updated() {
    let l = app_with_db_url(false);
    // Nothing recorded: the producer exists but its output was never recorded.
    let o = observed()
        .with(ours(node(&l, "db/run")))
        .with(ours_bound(node(&l, "app/run"), "db-url-1"));
    assert_eq!(steps(&run(&l, &o))[1], ("app/run", &Action::Update));
    // Nothing there at all: both are created.
    let p = run(&l, &observed());
    assert_eq!(
        steps(&p)[..2],
        [("db/run", &Action::Create), ("app/run", &Action::Create)]
    );
}

#[test]
fn consumer_is_updated_while_its_producer_is_changing() {
    let l = app_with_db_url(false);
    // The recorded output matches the object, but the producer is itself being updated, so
    // the value the consumer will see is not settled.
    let mut db = ours(node(&l, "db/run"));
    live_mut(&mut db).failed = true;
    let o = observed()
        .with_output("db", "URL", "db-url-1")
        .with(db)
        .with(ours_bound(node(&l, "app/run"), "db-url-1"));
    assert_eq!(
        steps(&run(&l, &o))[..2],
        [("db/run", &Action::Update), ("app/run", &Action::Update)]
    );
}

#[test]
fn literal_that_reads_like_a_reference_is_not_resolved_by_a_plan() {
    let l = app_with_db_url(true);
    let o = observed()
        .with_output("db", "URL", "db-url-1")
        .with(ours(node(&l, "db/run")))
        .with(ours(node(&l, "app/run")));
    // The literal is the object's value as it stands: nothing to do.
    let p = run(&l, &o);
    assert!(
        steps(&p).contains(&("app/run", &Action::Noop)),
        "{:?}",
        steps(&p)
    );
}

#[test]
fn plan_hash_tells_a_reference_from_a_literal_of_the_same_text() {
    let o = observed();
    assert_ne!(
        run(&app_with_db_url(true), &o).hash(),
        run(&app_with_db_url(false), &o).hash()
    );
}

#[test]
fn a_wanted_public_role_that_matches_is_a_noop() {
    let mut db = service("db", 5432);
    if let Some(Body::Service(s)) = db.body.as_mut() {
        s.exposure = Some(service::Exposure::Public(service::Public {}));
    }
    let l = lower(&ResourceList {
        resource: vec![db, service("db2", 1)],
    })
    .expect("the list lowers");
    let o = observed()
        .with(ours(node(&l, "db/run")))
        .with(ours(node(&l, "db/public")))
        .with(ours(node(&l, "db2/run")));
    let p = run(&l, &o);
    assert!(
        p.steps().iter().all(|s| s.action == Action::Noop),
        "{:?}",
        steps(&p)
    );
    assert!(steps(&p).iter().any(|(path, _)| *path == "db/public"));
}
