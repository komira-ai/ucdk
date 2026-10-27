// SPDX-License-Identifier: Apache-2.0
//! The rigs that run each kit against its fake, and the stubs the self-test runs it against. A
//! rig with no flaw is the fake; a rig with a flaw is the fake wrapped so that it breaks the one
//! promise a scenario holds it to, and no other.

#![allow(dead_code)]

use ucdk_release_seams::kit::Scenario;

pub mod build_system;
pub mod grant_source;
pub mod output_files;
pub mod process_runner;
pub mod publisher;
pub mod store;

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
