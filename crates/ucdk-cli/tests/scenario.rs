// SPDX-License-Identifier: Apache-2.0
//! The scenario every SDK repeats: plan, apply, plan again, destroy, over the fake provider with
//! the pair of services of `plan_apply_fake.txt`. The files in `tests/data` are what the command
//! line prints at each step; `sdk/python/tests/test_scenarios.py` reads the same files and holds
//! the SDK's outcomes to them (the same hash, the same steps, the same report), so the command
//! line and the SDKs are measured by one set of data.

mod common;

use common::{approve_and_run, cmd, pair, plan, scratch, write_list};
use ucdk_cli::World;

#[test]
fn the_scenario_prints_the_recorded_text_at_every_step() {
    let dir = scratch("the_scenario_prints_the_recorded_text_at_every_step");
    let input = write_list(&dir, "list.pb", pair());
    let saved = dir.join("saved.plan").to_string_lossy().into_owned();
    let mut world = World::new();

    let (planned, _) = plan(&mut world, "fake", "apply", &input, &saved);
    assert_eq!(planned.out, include_str!("data/plan_apply_fake.txt"));

    let status = format!("--input={input}");
    let before = cmd(&mut world, &["status", "--provider=fake", &status]);
    assert_eq!(before.out, include_str!("data/status_before_fake.txt"));

    let applied = approve_and_run(&mut world, "apply", "fake", &input, &dir);
    assert_eq!(applied.out, include_str!("data/apply_fake.txt"));

    let after = cmd(&mut world, &["status", "--provider=fake", &status]);
    assert_eq!(after.out, include_str!("data/status_fake.txt"));

    let (again, _) = plan(&mut world, "fake", "apply", &input, &saved);
    assert_eq!(again.out, include_str!("data/plan_again_fake.txt"));

    let (doomed, _) = plan(&mut world, "fake", "destroy", &input, &saved);
    assert_eq!(doomed.out, include_str!("data/plan_destroy_fake.txt"));

    let destroyed = approve_and_run(&mut world, "destroy", "fake", &input, &dir);
    assert_eq!(destroyed.out, include_str!("data/destroy_fake.txt"));

    let (gone, _) = plan(&mut world, "fake", "destroy", &input, &saved);
    assert_eq!(gone.out, include_str!("data/plan_destroyed_fake.txt"));
}
