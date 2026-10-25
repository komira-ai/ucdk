// SPDX-License-Identifier: Apache-2.0
//! The `status` command: it prints what the ledger records beside what the provider holds, it
//! changes nothing, and it exits with the class the one table in `ucdk-api` gives.

mod common;

use common::{
    approve_and_run, cmd, job_of, mutations, objects, pair, records, scratch, write_list,
    write_text,
};
use ucdk_api::v1::ExitClass;
use ucdk_cli::World;
use ucdk_ledger::Ledger as _;
use ucdk_provider::{Kind, PhysicalId, Provider as _, ProviderError, Status};

fn status(world: &mut World, provider: &str, input: &str) -> common::Ran {
    cmd(
        world,
        &[
            "status",
            &format!("--provider={provider}"),
            &format!("--input={input}"),
        ],
    )
}

fn id_of(world: &World, path: &str) -> PhysicalId {
    records(world)
        .into_iter()
        .find(|(p, _)| p.as_str() == path)
        .and_then(|(_, r)| r.physical)
        .expect("the record names an object")
}

const APPLIED: &str = "ucdk status\nprovider: fake\n\
  \x20\x20example-db/run [service] record: confirmed, live: ready, agrees\n\
  \x20\x20example-svc/public [public] record: confirmed, live: ready, agrees\n\
  \x20\x20example-svc/run [service] record: confirmed, live: ready, agrees\n\
3 objects\n";

/// Proves: after an apply, `status` prints one line per node with a confirmed record and a
/// ready object, in path order, and exits with the class "done". Catches a status text that
/// is built from the plan, or an exit other than the table's.
#[test]
fn status_after_apply_shows_every_node_confirmed() {
    let dir = scratch("status_after_apply_shows_every_node_confirmed");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();
    assert_eq!(
        approve_and_run(&mut world, "apply", "fake", &input, &dir).code,
        0
    );
    let ran = status(&mut world, "fake", &input);
    assert_eq!(ran.out, APPLIED);
    assert_eq!((ran.code, ran.err.as_str()), (ExitClass::Done.code(), ""));
}

/// Proves: on a fresh world every node is "not applied" and the command still exits done.
/// Catches a status that fails, or prints nothing, when the ledger is empty.
#[test]
fn status_on_an_empty_ledger_says_not_applied() {
    let dir = scratch("status_on_an_empty_ledger_says_not_applied");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();
    let ran = status(&mut world, "fake", &input);
    assert_eq!(
        ran.out,
        "ucdk status\nprovider: fake\n\
  \x20\x20example-db/run [service] record: none, live: absent, not applied\n\
  \x20\x20example-svc/public [public] record: none, live: absent, not applied\n\
  \x20\x20example-svc/run [service] record: none, live: absent, not applied\n\
3 objects\n"
    );
    assert_eq!(ran.code, ExitClass::Done.code());
}

/// Proves: an out-of-band delete shows as missing, a failed and a converging object show their
/// states, and a missing object is still exit done (it is a finding). Catches a status that
/// exits non-zero on drift, which would make it unusable in a script that reads the text.
#[test]
fn status_shows_a_missing_a_failed_and_a_converging_object() {
    let dir = scratch("status_shows_a_missing_a_failed_and_a_converging_object");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();
    approve_and_run(&mut world, "apply", "fake", &input, &dir);
    let gone = id_of(&world, "example-db/run");
    world.fake.delete(&Kind::service(), &gone).expect("delete");
    let failed = id_of(&world, "example-svc/run");
    assert!(world.fake.set_status(&failed, Status::Failed));
    let waiting = id_of(&world, "example-svc/public");
    assert!(world.fake.set_status(&waiting, Status::Converging));

    let ran = status(&mut world, "fake", &input);
    assert_eq!(
        ran.out,
        "ucdk status\nprovider: fake\n\
  \x20\x20example-db/run [service] record: confirmed, live: absent, missing (recorded, and the object is gone)\n\
  \x20\x20example-svc/public [public] record: confirmed, live: converging, agrees\n\
  \x20\x20example-svc/run [service] record: confirmed, live: failed, agrees\n\
3 objects\n"
    );
    assert_eq!(ran.code, ExitClass::Done.code());
}

/// Proves: status changes nothing: the ledger document and the objects are as they were and
/// the fake logged no mutating call. Catches a status that repairs, reaps or confirms.
#[test]
fn status_changes_nothing() {
    let dir = scratch("status_changes_nothing");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();
    approve_and_run(&mut world, "apply", "fake", &input, &dir);
    let gone = id_of(&world, "example-db/run");
    world.fake.delete(&Kind::service(), &gone).expect("delete");
    let document = world.ledger.read().expect("read");
    let before = (objects(&world.fake), mutations(&world.fake));
    status(&mut world, "fake", &input);
    assert_eq!(world.ledger.read().expect("read"), document);
    assert_eq!((objects(&world.fake), mutations(&world.fake)), before);
}

/// Proves: a kind the provider does not host is refused, as it is for plan and apply, with the
/// exit class refused. Catches a status that asks a provider about a kind it cannot hold.
#[test]
fn status_of_a_kind_the_provider_does_not_host_is_refused() {
    let dir = scratch("status_of_a_kind_the_provider_does_not_host_is_refused");
    let input = write_list(&dir, "list.pb", vec![job_of("nightly")]);
    let mut world = World::new();
    let ran = status(&mut world, "fake-limited", &input);
    assert_eq!(ran.code, ExitClass::Refused.code());
    assert_eq!(
        ran.out,
        "ucdk status\nprovider: fake-limited\nmode: apply\n\
refused: kind job is not hosted by fake-limited (nightly/run)\n"
    );
}

/// Proves: a provider read that fails ends the command as "failed, no effect" (exit 4) with
/// the advice that it is safe to run again, and prints no partial table. Catches a status that
/// swallows the failure or advises a re-plan.
#[test]
fn a_failed_read_exits_4_and_is_safe_to_repeat() {
    let dir = scratch("a_failed_read_exits_4_and_is_safe_to_repeat");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();
    world
        .fake
        .faults_mut()
        .fail_reads
        .push(ProviderError::Unknown("timed out".to_owned()));
    let ran = status(&mut world, "fake", &input);
    assert_eq!(ran.code, ExitClass::FailedNoEffect.code());
    assert_eq!(ran.out, "");
    assert_eq!(
        ran.err,
        "ucdk: failed: provider: unknown: timed out\nnothing was changed; it is safe to run again\n"
    );
}

/// Proves: usage mistakes and bad input exit with the table's classes: a flag status does not
/// take (`--mode`), a missing flag, an unreadable file (usage, 2) and a list with findings
/// (refused, 3). Catches a verb that accepts the plan flags or a wrong code.
#[test]
fn status_usage_and_input_errors_exit_with_the_table_classes() {
    let dir = scratch("status_usage_and_input_errors_exit_with_the_table_classes");
    let mut world = World::new();
    let ran = cmd(
        &mut world,
        &["status", "--provider=fake", "--input=a", "--mode=apply"],
    );
    assert_eq!(ran.code, ExitClass::Usage.code());
    assert!(
        ran.err
            .starts_with("ucdk: `--mode` is not a flag of this command\n"),
        "{}",
        ran.err
    );
    assert!(ran.err.contains("ucdk status --provider="), "{}", ran.err);
    let ran = cmd(&mut world, &["status", "--provider=fake"]);
    assert_eq!(ran.code, ExitClass::Usage.code());
    assert!(ran.err.contains("`--input` is needed"), "{}", ran.err);
    let ran = status(&mut world, "fake", &dir.join("none.pb").to_string_lossy());
    assert_eq!(ran.code, ExitClass::Usage.code());
    let bad = write_text(&dir, "bad.pb", "not a list");
    assert_eq!(
        status(&mut world, "fake", &bad).code,
        ExitClass::Usage.code()
    );
    let twice = write_list(
        &dir,
        "twice.pb",
        vec![
            common::service_of("a", false, &[]),
            common::service_of("a", false, &[]),
        ],
    );
    let ran = status(&mut world, "fake", &twice);
    assert_eq!(ran.code, ExitClass::Refused.code());
    assert_eq!(mutations(&world.fake), 0);
}

/// Proves: an interrupted run (an intent record beside a live object), a delete that began (a
/// reaping record beside a live object) and a foreign object under a wanted name each print
/// their own words, and none of them changes the exit class. Catches two states sharing text.
#[test]
fn status_words_an_interrupted_run_and_a_foreign_object() {
    let dir = scratch("status_words_an_interrupted_run_and_a_foreign_object");
    let input = write_list(&dir, "list.pb", pair());

    let mut world = World::new();
    world.fake.faults_mut().ambiguous_at_call.replace(1);
    let run = approve_and_run(&mut world, "apply", "fake", &input, &dir);
    assert_eq!(run.code, ExitClass::FailedChanged.code(), "{run:?}");
    let ran = status(&mut world, "fake", &input);
    assert!(
        ran.out.contains(
            "  example-db/run [service] record: intent, live: ready, pending (a run began and did not finish)\n"
        ),
        "{}",
        ran.out
    );
    assert_eq!(ran.code, ExitClass::Done.code());

    let mut world = World::new();
    approve_and_run(&mut world, "apply", "fake", &input, &dir);
    let snapshot = world.ledger.read().expect("read");
    let record = snapshot.records.values().next().expect("a record").clone();
    let path = record.path.as_str().to_owned();
    world
        .ledger
        .write(
            &snapshot.token,
            ucdk_ledger::Change::Put(record.in_state(ucdk_ledger::State::Reaping)),
        )
        .expect("write");
    let ran = status(&mut world, "fake", &input);
    assert!(
        ran.out.contains(&format!(
            "  {path} [service] record: reaping, live: ready, pending (a run began and did not finish)\n"
        )),
        "{}",
        ran.out
    );

    let mut world = World::new();
    world
        .fake
        .add_foreign(&Kind::service(), "example-db-run", None);
    let ran = status(&mut world, "fake", &input);
    assert!(
        ran.out.contains(
            "  example-db/run [service] record: none, live: ready, unclaimed (not ours, or ours with no record)\n"
        ),
        "{}",
        ran.out
    );
    assert_eq!(ran.code, ExitClass::Done.code());
}
