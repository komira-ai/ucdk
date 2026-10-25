// SPDX-License-Identifier: Apache-2.0
//! The first end-to-end runs: the verbs, through the library, over one in-memory world.

mod common;

use std::collections::BTreeSet;

use common::{
    approve_and_run, carry_out, cmd, job_of, mutations, objects, pair, plan, reading,
    recorded_paths, records, scratch, service_of, with_port, write_list, write_text,
};
use ucdk_cli::World;
use ucdk_provider::{Kind, Provider as _, ProviderError};
use ucdk_provider_fake::{Fake, FakeConfig, Faults, ForeignObject};

/// A name, the fault to arm, and the exit code expected.
type Case = (&'static str, fn(&mut Fake), u8);

fn untouched(world: &World) -> bool {
    mutations(&world.fake) == 0
        && mutations(&world.fake_limited) == 0
        && world.fake.object_count() == 0
        && world.fake_limited.object_count() == 0
        && recorded_paths(world).is_empty()
}

#[test]
fn plan_golden_output() {
    let dir = scratch("plan_golden_output");
    let input = write_list(&dir, "list.pb", pair());
    let saved = dir.join("saved.plan").to_string_lossy().into_owned();
    let mut world = World::new();

    let (ran, _) = plan(&mut world, "fake", "apply", &input, &saved);

    assert_eq!(ran.code, 0, "{ran:?}");
    assert_eq!(ran.err, "");
    assert_eq!(ran.out, include_str!("data/plan_apply_fake.txt"));
    assert_eq!(std::fs::read_to_string(&saved).expect("saved"), ran.out);
    assert!(
        untouched(&world),
        "a plan reads the world and changes nothing"
    );
}

#[test]
fn limited_provider_refuses_job_exit_3_creates_nothing() {
    let dir = scratch("limited_provider_refuses_job_exit_3_creates_nothing");
    let input = write_list(
        &dir,
        "list.pb",
        vec![
            service_of("example-svc", false, &[]),
            job_of("example-batch"),
        ],
    );
    let saved = dir.join("saved.plan").to_string_lossy().into_owned();
    let mut world = World::new();

    let (planned, _) = plan(&mut world, "fake-limited", "apply", &input, &saved);
    assert_eq!(planned.code, 3, "{planned:?}");
    assert_eq!(
        planned.out,
        include_str!("data/plan_apply_fake_limited_refused.txt")
    );

    // An apply under a plan that the fake cloud would have accepted is refused the same way.
    let (_, hash) = plan(&mut world, "fake", "apply", &input, &saved);
    let by_hand = write_text(
        &dir,
        "by-hand.plan",
        &format!("ucdk plan\nprovider: fake-limited\nmode: apply\nplan-hash: {hash}\n"),
    );
    let applied = carry_out(&mut world, "apply", "fake-limited", &input, &by_hand, &hash);
    assert_eq!(applied.code, 3, "{applied:?}");
    assert!(
        applied
            .out
            .contains("refused: kind job is not hosted by fake-limited (example-batch/run)")
    );
    assert!(untouched(&world), "not even the service was created");

    // The same file without the job is hosted.
    let services = write_list(
        &dir,
        "services.pb",
        vec![service_of("example-svc", false, &[])],
    );
    let ran = approve_and_run(&mut world, "apply", "fake-limited", &services, &dir);
    assert_eq!(ran.code, 0, "{ran:?}");
    assert_eq!(world.fake_limited.object_count(), 1);
}

#[test]
fn apply_then_reapply_reports_no_changes() {
    let dir = scratch("apply_then_reapply_reports_no_changes");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();

    let first = approve_and_run(&mut world, "apply", "fake", &input, &dir);
    assert_eq!(first.code, 0, "{first:?}");
    assert_eq!(
        first.out,
        "  created  example-db/run\n  created  example-svc/run\n  created  example-svc/public\n3 changed\n"
    );
    let objects_after = objects(&world.fake);
    let mutated = mutations(&world.fake);
    let records_after = recorded_paths(&world);

    let again = approve_and_run(&mut world, "apply", "fake", &input, &dir);
    assert_eq!(
        (again.code, again.out.as_str()),
        (0, "no changes\n"),
        "{again:?}"
    );
    assert_eq!(objects(&world.fake), objects_after);
    assert_eq!(mutations(&world.fake), mutated);
    assert_eq!(recorded_paths(&world), records_after);
    assert_eq!(records_after.len(), 3);
}

#[test]
fn apply_rejects_plan_with_wrong_hash() {
    let dir = scratch("apply_rejects_plan_with_wrong_hash");
    let input = write_list(&dir, "list.pb", pair());
    let other = write_list(
        &dir,
        "other.pb",
        vec![service_of("example-other", false, &[])],
    );
    let saved = dir.join("saved.plan").to_string_lossy().into_owned();
    let mut world = World::new();
    let (_, hash) = plan(&mut world, "fake", "apply", &input, &saved);
    let (_, other_hash) = plan(
        &mut world,
        "fake",
        "apply",
        &other,
        &dir.join("o.plan").to_string_lossy(),
    );

    // A hash that the saved plan does not carry.
    let wrong = "a".repeat(64);
    let ran = carry_out(&mut world, "apply", "fake", &input, &saved, &wrong);
    assert_eq!(ran.code, 3, "{ran:?}");
    assert!(
        ran.err.contains("is not the saved apply plan"),
        "{}",
        ran.err
    );
    assert!(untouched(&world));

    // The saved plan was made for another file: the hash is the saved plan's own, but it is not
    // the plan of this input.
    let other_saved = dir.join("o.plan").to_string_lossy().into_owned();
    let ran = carry_out(
        &mut world,
        "apply",
        "fake",
        &input,
        &other_saved,
        &other_hash,
    );
    assert_eq!(ran.code, 3, "{ran:?}");
    assert!(ran.err.contains("the plan is "), "{}", ran.err);
    assert!(untouched(&world));

    // A plan saved for another provider or mode is not the plan to run.
    let ran = carry_out(&mut world, "apply", "fake-limited", &input, &saved, &hash);
    assert_eq!(ran.code, 3, "{ran:?}");
    let ran = carry_out(&mut world, "destroy", "fake", &input, &saved, &hash);
    assert_eq!(ran.code, 3, "{ran:?}");
    assert!(untouched(&world));

    // The right pair runs.
    let ran = carry_out(&mut world, "apply", "fake", &input, &saved, &hash);
    assert_eq!(ran.code, 0, "{ran:?}");
}

#[test]
fn crash_mid_apply_then_rerun_converges() {
    let dir = scratch("crash_mid_apply_then_rerun_converges");
    let input = write_list(&dir, "list.pb", pair());
    let mut clean = World::new();
    assert_eq!(
        approve_and_run(&mut clean, "apply", "fake", &input, &dir).code,
        0
    );
    let clean_objects = objects(&clean.fake);
    assert_eq!(clean.fake.object_count(), 3);

    // A mutation is refused and nothing had changed before it: safe to run again (4); one
    // refused after another landed, or whose answer was lost, may have changed things (6).
    let cases: [Case; 4] = [
        (
            "refused first",
            |f| *f.faults_mut().fail_at_call = Some(1),
            4,
        ),
        (
            "refused second",
            |f| *f.faults_mut().fail_at_call = Some(2),
            6,
        ),
        (
            "lost answer after one landed",
            |f| *f.faults_mut().ambiguous_at_call = Some(2),
            6,
        ),
        (
            "lost answer of the first",
            |f| *f.faults_mut().ambiguous_at_call = Some(1),
            6,
        ),
    ];
    for (name, arm, code) in cases {
        let mut world = World::new();
        arm(&mut world.fake);

        let crashed = approve_and_run(&mut world, "apply", "fake", &input, &dir);
        assert_eq!(crashed.code, code, "{name}: {crashed:?}");
        assert!(
            crashed.err.contains("failed at "),
            "{name}: {}",
            crashed.err
        );
        assert!(world.fake.object_count() < 4, "{name}");

        let rerun = approve_and_run(&mut world, "apply", "fake", &input, &dir);
        assert_eq!(rerun.code, 0, "{name}: {rerun:?}");
        assert_eq!(
            objects(&world.fake),
            clean_objects,
            "{name}: no duplicates, no orphans"
        );
        assert_eq!(recorded_paths(&world), recorded_paths(&clean), "{name}");

        let settled = approve_and_run(&mut world, "apply", "fake", &input, &dir);
        assert_eq!(
            (settled.code, settled.out.as_str()),
            (0, "no changes\n"),
            "{name}"
        );
    }
}

#[test]
fn ambiguous_read_does_not_reap() {
    let dir = scratch("ambiguous_read_does_not_reap");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();
    assert_eq!(
        approve_and_run(&mut world, "apply", "fake", &input, &dir).code,
        0
    );
    let objects_before = objects(&world.fake);
    let records_before = recorded_paths(&world);

    // A read that fails, or cannot tell, is not "the object is gone": nothing is deleted or
    // reaped, and the records stay.
    for error in [
        ProviderError::Transient("the read timed out".to_owned()),
        ProviderError::Unknown("the answer was lost".to_owned()),
    ] {
        // The read fails while the plan is being made.
        world.fake.faults_mut().fail_reads.push(error.clone());
        let saved = dir.join("destroy.plan").to_string_lossy().into_owned();
        let (planned, _) = plan(&mut world, "fake", "destroy", &input, &saved);
        assert_eq!(planned.code, 4, "{planned:?}");
        assert!(
            planned.err.contains("nothing was changed"),
            "{}",
            planned.err
        );

        // The read fails after the plan was approved, as the run starts.
        let (_, hash) = plan(&mut world, "fake", "destroy", &input, &saved);
        world.fake.faults_mut().fail_reads.push(error);
        let ran = carry_out(&mut world, "destroy", "fake", &input, &saved, &hash);
        assert_eq!(ran.code, 4, "{ran:?}");
        assert!(ran.err.contains("nothing was changed"), "{}", ran.err);
        assert_eq!(objects(&world.fake), objects_before);
        assert_eq!(recorded_paths(&world), records_before);
        assert_eq!(mutations(&world.fake), 3);
    }

    // Once reads answer, the destroy removes the objects it owns and drops their records.
    let ran = approve_and_run(&mut world, "destroy", "fake", &input, &dir);
    assert_eq!(ran.code, 0, "{ran:?}");
    assert_eq!(world.fake.object_count(), 0);
    assert!(recorded_paths(&world).is_empty());
}

#[test]
fn foreign_resource_exits_3_creates_nothing() {
    let dir = scratch("foreign_resource_exits_3_creates_nothing");
    let input = write_list(&dir, "list.pb", pair());
    let saved = dir.join("saved.plan").to_string_lossy().into_owned();
    let (_, hash) = plan(&mut World::new(), "fake", "apply", &input, &saved);

    let foreign = |marker: Option<&str>| FakeConfig {
        faults: Faults {
            foreign: vec![ForeignObject {
                kind: Kind::service(),
                name: "example-db-run".to_owned(),
                marker: marker.map(str::to_owned),
            }],
            ..Faults::default()
        },
        ..FakeConfig::standard()
    };
    for marker in [None, Some("another-tool")] {
        let mut world = World::new();
        world.fake = Fake::new(foreign(marker));

        let planned = cmd(
            &mut world,
            &["plan", "--provider=fake", &format!("--input={input}")],
        );
        assert_eq!(planned.code, 3, "{planned:?}");
        assert!(planned.err.contains("example-db/run"), "{}", planned.err);

        let applied = carry_out(&mut world, "apply", "fake", &input, &saved, &hash);
        assert_eq!(applied.code, 3, "{applied:?}");
        assert!(applied.err.contains("example-db/run"), "{}", applied.err);

        assert_eq!(
            world.fake.object_count(),
            1,
            "only the foreign object is there"
        );
        assert_eq!(mutations(&world.fake), 0);
        assert!(recorded_paths(&world).is_empty());
    }
}

#[test]
fn destroy_removes_only_owned_and_skips_keep() {
    let dir = scratch("destroy_removes_only_owned_and_skips_keep");
    let input = write_list(
        &dir,
        "list.pb",
        vec![
            service_of("example-svc", false, &["example-db"]),
            service_of("example-db", false, &[]),
            job_of("example-batch"),
        ],
    );
    // The jobs of this cloud hold data, so removing one keeps it; and an object the file does
    // not name sits in the cloud from the start.
    let mut world = World::new();
    world.fake = Fake::new(FakeConfig {
        stateful: BTreeSet::from([Kind::job()]),
        faults: Faults {
            foreign: vec![ForeignObject {
                kind: Kind::service(),
                name: "example-unrelated".to_owned(),
                marker: None,
            }],
            ..Faults::default()
        },
        ..FakeConfig::standard()
    });
    assert_eq!(
        approve_and_run(&mut world, "apply", "fake", &input, &dir).code,
        0
    );
    assert_eq!(world.fake.object_count(), 4);

    let ran = approve_and_run(&mut world, "destroy", "fake", &input, &dir);

    assert_eq!(ran.code, 0, "{ran:?}");
    assert!(
        ran.out.contains("  deleted  example-svc/run\n"),
        "{}",
        ran.out
    );
    assert!(
        ran.out.contains("  deleted  example-db/run\n"),
        "{}",
        ran.out
    );
    assert!(
        ran.out
            .contains("  retained example-batch/run (it holds data)\n"),
        "{}",
        ran.out
    );
    assert_eq!(
        world.fake.object_count(),
        2,
        "the kept job and the unrelated object remain"
    );
    assert_eq!(recorded_paths(&world), ["example-batch/run"]);

    let again = approve_and_run(&mut world, "destroy", "fake", &input, &dir);
    assert_eq!(again.code, 0, "{again:?}");
    assert!(again.out.ends_with("no changes\n"), "{}", again.out);
    assert_eq!(world.fake.object_count(), 2);
}

#[test]
fn plan_shows_update_reap_and_each_reason_for_keeping() {
    let dir = scratch("plan_shows_update_reap_and_each_reason_for_keeping");
    let first = write_list(
        &dir,
        "first.pb",
        vec![
            service_of("example-a", true, &[]),
            service_of("example-b", false, &[]),
        ],
    );
    let mut world = World::new();
    world.fake = Fake::new(FakeConfig {
        deletion_forbidden: BTreeSet::from([Kind::new("public")]),
        ..FakeConfig::standard()
    });
    assert_eq!(
        approve_and_run(&mut world, "apply", "fake", &first, &dir).code,
        0
    );

    // An object carries a field the catalog does not model, so the provider will not delete it.
    let physical = |world: &World, path: &str| {
        records(world)
            .into_iter()
            .find(|(p, _)| p.as_str() == path)
            .and_then(|(_, record)| record.physical)
            .expect("the path is recorded")
    };
    let public = physical(&world, "example-a/public");
    assert!(world.fake.set_unmodelled(&public, "note", "kept by hand"));

    // The file now lists only `example-a`, without its public role and on another port.
    let second = write_list(
        &dir,
        "second.pb",
        vec![with_port(service_of("example-a", false, &[]), 9090)],
    );
    let saved = dir.join("second.plan").to_string_lossy().into_owned();
    let (planned, _) = plan(&mut world, "fake", "apply", &second, &saved);
    assert_eq!(planned.code, 0, "{planned:?}");
    for line in [
        "  update  example-a/run [service]\n",
        "  retain  example-a/public [public] (it cannot be deleted: ",
        "  retain  example-b/run [service] (the file no longer names its owner)\n",
    ] {
        assert!(planned.out.contains(line), "{line}: {}", planned.out);
    }
    assert!(planned.out.ends_with("1 to change\n"), "{}", planned.out);

    // An object gone from the cloud, with its record still there, is reaped.
    let run = physical(&world, "example-b/run");
    world
        .fake
        .delete(&Kind::service(), &run)
        .expect("delete out of band");
    let (planned, hash) = plan(&mut world, "fake", "destroy", &first, &saved);
    assert!(
        planned
            .out
            .contains("  reap    example-b/run [service] (already gone; the record is dropped)\n"),
        "{}",
        planned.out
    );
    let ran = carry_out(&mut world, "destroy", "fake", &first, &saved, &hash);
    assert_eq!(ran.code, 0, "{ran:?}");
    assert!(
        ran.out.contains("  reaped   example-b/run\n"),
        "{}",
        ran.out
    );
}

#[test]
fn a_new_world_is_empty() {
    let world = World::default();
    assert_eq!(
        world.fake.object_count() + world.fake_limited.object_count(),
        0
    );
    assert!(recorded_paths(&world).is_empty());
    assert_eq!((world.out.as_str(), world.err.as_str()), ("", ""));
}

#[test]
fn render_known_after_apply() {
    let dir = scratch("render_known_after_apply");
    let input = write_list(
        &dir,
        "list.pb",
        vec![
            reading("example-svc", "example-db"),
            service_of("example-db", false, &[]),
        ],
    );
    let saved = dir.join("saved.plan").to_string_lossy().into_owned();
    let mut world = World::new();

    let (ran, _) = plan(&mut world, "fake", "apply", &input, &saved);

    assert_eq!(ran.code, 0, "{ran:?}");
    let lines: Vec<&str> = ran.out.lines().skip(4).collect();
    assert_eq!(
        lines,
        [
            "  create  example-db/run [service]",
            "  later   example-svc/run [service] (reads a value that is known only after apply)",
            "2 to change",
        ]
    );
}

#[test]
fn second_apply_of_a_list_with_a_ref_prints_no_changes() {
    let dir = scratch("second_apply_of_a_list_with_a_ref");
    let input = write_list(
        &dir,
        "list.pb",
        vec![
            reading("example-svc", "example-db"),
            service_of("example-db", false, &[]),
        ],
    );
    let mut world = World::new();
    let first = approve_and_run(&mut world, "apply", "fake", &input, &dir);
    assert_eq!(first.code, 0, "{first:?}");
    let saved = dir.join("again.plan").to_string_lossy().into_owned();
    let (ran, _) = plan(&mut world, "fake", "apply", &input, &saved);
    assert!(ran.out.ends_with("no changes\n"), "{}", ran.out);
    assert!(!ran.out.contains("later"), "{}", ran.out);
}

/// Proves: an apply on a ledger another holder has leased exits 3 (refused), says
/// who holds it on standard error, and creates nothing; a plan and a status are
/// answered anyway; once the holder releases, the same apply goes through. Catches a command
/// that writes under someone else's lease, and one that reports the lease as a crash (exit 6).
#[test]
fn a_ledger_leased_to_another_holder_refuses_apply_with_exit_3() {
    let dir = scratch("a_ledger_leased_to_another_holder_refuses_apply_with_exit_3");
    let input = write_list(&dir, "list.pb", pair());
    let saved = dir.join("saved.plan").to_string_lossy().into_owned();
    let mut world = World::new();
    let (_, hash) = plan(&mut world, "fake", "apply", &input, &saved);
    let lease =
        ucdk_ledger::Ledger::acquire(&world.ledger, "ci-7", std::time::Duration::from_secs(600))
            .expect("another holder takes the lease");

    let refused = carry_out(&mut world, "apply", "fake", &input, &saved, &hash);
    assert_eq!(refused.code, 3, "{refused:?}");
    assert!(
        refused.err.contains("the ledger is leased to ci-7"),
        "{refused:?}"
    );
    assert!(untouched(&world), "a refused apply changes nothing");

    let doomed_plan = dir.join("doomed.plan").to_string_lossy().into_owned();
    let (doomed, _) = plan(&mut world, "fake", "destroy", &input, &doomed_plan);
    assert_eq!(doomed.code, 0, "a plan needs no lease: {doomed:?}");
    let status = cmd(
        &mut world,
        &["status", "--provider=fake", &format!("--input={input}")],
    );
    assert_eq!(status.code, 0, "a status needs no lease: {status:?}");

    ucdk_ledger::Ledger::release(&world.ledger, lease.fence).expect("release");
    let applied = carry_out(&mut world, "apply", "fake", &input, &saved, &hash);
    assert_eq!(applied.code, 0, "{applied:?}");
}
