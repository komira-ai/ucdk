// SPDX-License-Identifier: Apache-2.0
//! Status through the command line: a failed object is updated, a converging one is waited
//! for with exit 0 and a note, and a kind with no stamp of its own is refused.

mod common;

use common::{approve_and_run, cmd, mutations, pair, plan, records, scratch, write_list};
use ucdk_cli::World;
use ucdk_provider::{Kind, Status};
use ucdk_provider_fake::{Fake, FakeConfig};

fn id_of(world: &World, path: &str) -> ucdk_provider::PhysicalId {
    records(world)
        .into_iter()
        .find(|(p, _)| p.as_str() == path)
        .and_then(|(_, r)| r.physical)
        .expect("the record names an object")
}

#[test]
fn a_failed_object_is_planned_and_applied_as_an_update() {
    let dir = scratch("a_failed_object_is_planned_and_applied_as_an_update");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();
    assert_eq!(
        approve_and_run(&mut world, "apply", "fake", &input, &dir).code,
        0
    );
    let first = records(&world)
        .first()
        .map(|(p, _)| p.as_str().to_owned())
        .expect("a record");
    let id = id_of(&world, &first);
    assert!(world.fake.set_status(&id, Status::Failed));

    let saved = dir.join("again.plan").to_string_lossy().into_owned();
    let (planned, _) = plan(&mut world, "fake", "apply", &input, &saved);
    assert_eq!(planned.code, 0, "{planned:?}");
    assert!(
        planned.out.contains(&format!("  update  {first} ")),
        "{}",
        planned.out
    );
    assert!(planned.out.contains("1 to change\n"), "{}", planned.out);

    let ran = approve_and_run(&mut world, "apply", "fake", &input, &dir);
    assert_eq!(ran.code, 0, "{ran:?}");
    assert!(
        ran.out.contains(&format!("  updated  {first}\n")),
        "{}",
        ran.out
    );
    assert!(!world.fake.dump().contains("status=failed"));
}

#[test]
fn a_converging_object_exits_zero_with_a_note_and_changes_nothing() {
    let dir = scratch("a_converging_object_exits_zero_with_a_note_and_changes_nothing");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();
    assert_eq!(
        approve_and_run(&mut world, "apply", "fake", &input, &dir).code,
        0
    );
    let first = records(&world)
        .first()
        .map(|(p, _)| p.as_str().to_owned())
        .expect("a record");
    let id = id_of(&world, &first);
    assert!(world.fake.set_status(&id, Status::Converging));
    let writes = mutations(&world.fake);

    let saved = dir.join("wait.plan").to_string_lossy().into_owned();
    let (planned, _) = plan(&mut world, "fake", "apply", &input, &saved);
    assert_eq!(planned.code, 0, "{planned:?}");
    assert!(
        planned.out.contains(&format!(
            "  wait    {first} [service] (still converging; nothing is written)\n"
        )),
        "{}",
        planned.out
    );
    assert!(planned.out.ends_with("no changes\n"), "{}", planned.out);

    let ran = approve_and_run(&mut world, "apply", "fake", &input, &dir);
    assert_eq!(ran.code, 0, "{ran:?}");
    assert_eq!(ran.err, "");
    assert!(
        ran.out.contains(&format!(
            "  waiting  {first} (still converging; its record is not confirmed until it is ready)\n"
        )),
        "{}",
        ran.out
    );
    assert_eq!(mutations(&world.fake), writes, "nothing was written");
}

#[test]
fn a_kind_with_no_stamp_of_its_own_exits_three() {
    let dir = scratch("a_kind_with_no_stamp_of_its_own_exits_three");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();
    world.fake = Fake::new(FakeConfig {
        embedded: [Kind::service()].into(),
        ..FakeConfig::standard()
    });
    let ran = cmd(
        &mut world,
        &[
            "plan",
            "--provider=fake",
            "--mode=apply",
            &format!("--input={input}"),
        ],
    );
    assert_eq!(ran.code, 3, "{ran:?}");
    assert!(
        ran.err
            .contains("refused: kinds that carry no stamp of their own are not applied yet: "),
        "{}",
        ran.err
    );
    assert_eq!(world.fake.object_count(), 0);
}

#[test]
fn an_object_born_failed_exits_zero_with_a_note_for_each_object() {
    let dir = scratch("an_object_born_failed_exits_zero_with_a_note_for_each_object");
    let input = write_list(&dir, "list.pb", pair());
    let mut world = World::new();
    *world.fake.faults_mut().create_failed = true;
    let ran = approve_and_run(&mut world, "apply", "fake", &input, &dir);
    assert_eq!(ran.code, 0, "{ran:?}");
    assert_eq!(ran.err, "");
    let paths: Vec<String> = records(&world)
        .into_iter()
        .map(|(p, _)| p.as_str().to_owned())
        .collect();
    assert!(!paths.is_empty());
    for path in paths {
        assert!(
            ran.out.contains(&format!(
                "  failed   {path} (still failed after the write; its record is not confirmed, run again to retry)\n"
            )),
            "{}",
            ran.out
        );
    }
}
