// SPDX-License-Identifier: Apache-2.0
//! The rigs that run each kit against its fake, and the stubs the self-test runs it against. A
//! rig with no flaw is the fake; a rig with a flaw is the fake wrapped so that it breaks the one
//! promise a scenario holds it to, and no other.

#![allow(dead_code)]

use ucdk_release_seams::kit::{Report, Scenario};

pub mod hostile;
pub mod loose;
pub mod store;

/// The names of the scenarios of a kit, in its order.
pub fn names<R>(scenarios: &[Scenario<R>]) -> Vec<&'static str> {
    scenarios.iter().map(|s| s.name).collect()
}

/// Holds that the table names every scenario of the kit, in its order, and that each stub fails
/// its scenario alone.
pub fn each_fails_alone<F: Copy>(
    kit: &str,
    scenarios: &[&'static str],
    table: &[(&'static str, F)],
    run: impl Fn(F) -> Report,
) {
    let named: Vec<&str> = table.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        named, scenarios,
        "{kit}: the table has a stub for each scenario, in order"
    );
    for (name, flaw) in table {
        let report = run(*flaw);
        assert_eq!(
            report.failed(),
            vec![*name],
            "{kit}: the stub for {name} must fail that scenario and no other: {:?}",
            report.outcomes
        );
    }
}

/// Holds that each further stub fails the scenarios it names, which are in the kit's order, and
/// no other. These are the wrong implementations that a first version of a scenario let pass.
pub fn further_stubs_fail_exactly<F: Copy + std::fmt::Debug>(
    kit: &str,
    scenarios: &[&'static str],
    table: &[(F, &[&'static str])],
    run: impl Fn(F) -> Report,
) {
    let mut wrong = Vec::new();
    for (flaw, named) in table {
        let in_order: Vec<&str> = scenarios
            .iter()
            .copied()
            .filter(|s| named.contains(s))
            .collect();
        assert_eq!(
            in_order.as_slice(),
            *named,
            "{kit}: {flaw:?} names scenarios the kit has, in the kit's order"
        );
        let failed = run(*flaw).failed();
        if failed != *named {
            wrong.push(format!("{flaw:?}: failed {failed:?}, named {named:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{kit}: each stub must fail exactly the scenarios it names:\n{}",
        wrong.join("\n")
    );
}

/// Runs the scenario called `name` against `rig`, and fails the test with its reason if it
/// fails.
pub fn run_named<R>(scenarios: &[Scenario<R>], name: &str, rig: &R) {
    let Some(scenario) = scenarios.iter().find(|s| s.name == name) else {
        panic!("the kit has no scenario called {name}");
    };
    if let Err(why) = scenario.run_on(rig) {
        panic!("{name} failed: {why}");
    }
}

/// One test for each scenario of a kit, named as the scenario is, each run against every rig of
/// `rigs`, and a test that the list names every scenario of the kit, in its order.
#[allow(unused_macros)]
macro_rules! kit_tests {
    ($rig:ty, $rigs:expr, $scenarios:ident; $($name:ident),+ $(,)?) => {
        $(
            #[test]
            fn $name() {
                for rig in $rigs {
                    $crate::rigs::run_named(&$scenarios::<$rig>(), stringify!($name), &rig);
                }
            }
        )+

        #[test]
        fn every_scenario_of_the_kit_has_its_test() {
            let named: Vec<&str> = vec![$(stringify!($name)),+];
            let kit: Vec<&str> = $scenarios::<$rig>().iter().map(|s| s.name).collect();
            assert_eq!(named, kit);
        }
    };
}
