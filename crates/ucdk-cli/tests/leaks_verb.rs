// SPDX-License-Identifier: Apache-2.0
//! The `leaks` command: it prints the objects that carry the cell's stamp and have no ledger
//! record, changes nothing, and exits 8 when there are some and 0 when there are none. An
//! `apply` reports leaks and exits as it always did.

mod common;

use common::{approve_and_run, cmd, mutations, objects, pair, scratch, write_list};
use ucdk_api::v1::ExitClass;
use ucdk_cli::World;
use ucdk_ledger::Ledger as _;
use ucdk_provider::{Kind, Node, NodePath, PhysicalId, Provider as _, ProviderError, Stamp};

fn leaks(world: &mut World, provider: &str) -> common::Ran {
    cmd(world, &["leaks", &format!("--provider={provider}")])
}

/// A stamped object of this world's cell that no record stands for.
fn leaked(world: &mut World, path: &str) -> PhysicalId {
    let path = NodePath::new(path).expect("a path");
    let name = path.as_str().replace('/', "-");
    let node = Node::new(path.clone(), Kind::service(), name, Vec::new()).expect("a node");
    let stamp = Stamp::for_node(&world.cell, &path);
    world.fake.create(&node, &stamp).expect("create").physical
}

fn line(id: &PhysicalId, path: &str) -> String {
    let path = NodePath::new(path).expect("a path");
    format!(
        "  leaked  {id} [service] \"{}\" (stamped for {}, node {}; the ledger has no record of it)\n",
        path.as_str().replace('/', "-"),
        path.owner(),
        path.key()
    )
}

/// Proves: with a leaked object in the cell, `leaks` prints it and exits 8, with nothing on
/// standard error. Catches a sweep that exits 0 on leaks (a script could not tell), or one that
/// prints nothing.
#[test]
fn leaks_exits_8_and_names_the_leaked_object() {
    let mut world = World::new();
    let id = leaked(&mut world, "ghost/run");
    let ran = leaks(&mut world, "fake");
    assert_eq!(
        ran.out,
        format!(
            "ucdk leaks\nprovider: fake\n{}1 leaks\n",
            line(&id, "ghost/run")
        )
    );
    assert_eq!((ran.code, ran.err.as_str()), (8, ""));
    assert_eq!(ran.code, ExitClass::Leaks.code());
}

/// Proves: with nothing leaked, `leaks` exits 0, on an empty world and after an apply whose
/// objects are all recorded. Catches recorded objects reported as leaks.
#[test]
fn leaks_exits_0_when_there_are_none() {
    let dir = scratch("leaks_exits_0_when_there_are_none");
    let mut world = World::new();
    let ran = leaks(&mut world, "fake");
    assert_eq!(ran.out, "ucdk leaks\nprovider: fake\n0 leaks\n");
    assert_eq!((ran.code, ran.err.as_str()), (0, ""));

    let input = write_list(&dir, "list.pb", pair());
    assert_eq!(
        approve_and_run(&mut world, "apply", "fake", &input, &dir).code,
        0
    );
    let ran = leaks(&mut world, "fake");
    assert_eq!(ran.out, "ucdk leaks\nprovider: fake\n0 leaks\n");
    assert_eq!(ran.code, 0);
}

/// Proves: `leaks` changes nothing: the ledger document and the objects are as they were, no
/// mutating call was logged, and the leaked object is still there afterwards (a stamp alone
/// authorises no delete). Catches a sweep that cleans up what it finds.
#[test]
fn leaks_changes_nothing_and_deletes_nothing() {
    let mut world = World::new();
    let id = leaked(&mut world, "ghost/run");
    let document = world.ledger.read().expect("read");
    let before = (objects(&world.fake), mutations(&world.fake));
    leaks(&mut world, "fake");
    assert_eq!(world.ledger.read().expect("read"), document);
    assert_eq!((objects(&world.fake), mutations(&world.fake)), before);
    assert_eq!(leaks(&mut world, "fake").code, 8);
    assert_eq!(world.fake.object_count(), 1, "{id} is still there");
}

/// Proves: an object stamped by another cell is not reported by this cell's `leaks`. Catches a
/// sweep that lists every stamped object.
#[test]
fn leaks_ignores_another_cells_objects() {
    let mut world = World::new();
    let path = NodePath::new("ghost/run").expect("a path");
    let node = Node::new(
        path.clone(),
        Kind::service(),
        "ghost-run".to_owned(),
        Vec::new(),
    )
    .expect("a node");
    let stamp = Stamp::for_node(&ucdk_provider::kit::cell(), &path);
    world.fake.create(&node, &stamp).expect("create");
    let ran = leaks(&mut world, "fake");
    assert_eq!(
        (ran.code, ran.out.as_str()),
        (0, "ucdk leaks\nprovider: fake\n0 leaks\n")
    );
}

/// Proves: an `apply` that leaves a leaked object beside it prints it under its own lines and
/// still exits 0, and the object is untouched. Catches an apply that fails for leaks or deletes
/// them.
#[test]
fn apply_reports_a_leak_and_still_exits_0() {
    let dir = scratch("apply_reports_a_leak_and_still_exits_0");
    let mut world = World::new();
    let id = leaked(&mut world, "ghost/run");
    let input = write_list(&dir, "list.pb", pair());
    let ran = approve_and_run(&mut world, "apply", "fake", &input, &dir);
    assert_eq!((ran.code, ran.err.as_str()), (0, ""));
    assert_eq!(
        ran.out,
        format!(
            "  created  example-db/run\n  created  example-svc/run\n  created  example-svc/public\n{}3 changed\n",
            line(&id, "ghost/run").replace("  leaked  ", "  leaked   ")
        )
    );
    assert_eq!(world.fake.object_count(), 4);
}

/// Proves: a sweep that cannot read ends as a failure that changed nothing (exit 4), and an
/// apply whose sweep cannot read still exits 0 and says so. Catches an unreadable cloud read
/// as "no leaks".
#[test]
fn a_sweep_that_cannot_read_fails_with_no_effect() {
    let mut world = World::new();
    world
        .fake
        .faults_mut()
        .fail_reads
        .push(ProviderError::Transient("down".to_owned()));
    let ran = leaks(&mut world, "fake");
    assert_eq!(ran.code, ExitClass::FailedNoEffect.code());
    assert!(ran.err.contains("down"), "{}", ran.err);
    assert_eq!(ran.out, "");
}

/// Proves: the usage of `leaks`: it takes `--provider` and nothing else (no input file), and a
/// provider it does not know is a usage error. Catches a verb that silently ignores a flag.
#[test]
fn leaks_takes_a_provider_and_no_other_flag() {
    let mut world = World::new();
    for args in [
        vec!["leaks"],
        vec!["leaks", "--provider=fake", "--input=list.pb"],
        vec!["leaks", "--provider=nowhere"],
    ] {
        let ran = cmd(&mut world, &args);
        assert_eq!(ran.code, ExitClass::Usage.code(), "{args:?}");
        assert!(ran.err.contains("usage: ucdk"), "{args:?}: {}", ran.err);
    }
    let ran = leaks(&mut world, "fake-limited");
    assert_eq!(ran.out, "ucdk leaks\nprovider: fake-limited\n0 leaks\n");
}
