// SPDX-License-Identifier: Apache-2.0
//! The runner every kit shares: it names the scenarios that failed, a scenario that panics for
//! another reason is a failure and not a crash of the kit, and the report says which is which.

use std::cell::RefCell;

use ucdk_release_seams::kit::{Beats, Outcome, Report, Scenario, run_scenarios};

fn passes(_: &()) {}

fn panics(_: &()) {
    panic!("a bug in the implementation");
}

fn kit() -> Vec<Scenario<()>> {
    vec![
        Scenario::new("passes", passes),
        Scenario::new("panics", panics),
    ]
}

#[test]
fn a_scenario_that_panics_is_a_failed_scenario_and_the_others_still_run() {
    let report = run_scenarios(&(), &kit());
    assert_eq!(report.names(), vec!["passes", "panics"]);
    assert_eq!(report.failed(), vec!["panics"]);
    assert_eq!(
        report.outcomes[1].result,
        Err("the scenario panicked".to_owned())
    );
    assert_eq!(report.outcomes[0].result, Ok(()));
}

#[test]
#[should_panic(expected = "panics: the scenario panicked")]
fn a_report_with_a_failure_does_not_conform() {
    run_scenarios(&(), &kit()).assert_conforms();
}

#[test]
fn a_report_without_a_failure_conforms() {
    let report = Report {
        outcomes: vec![Outcome {
            name: "passes",
            result: Ok(()),
        }],
    };
    report.assert_conforms();
    assert_eq!(report.failed(), Vec::<&str>::new());
}

#[test]
fn a_scenario_can_be_run_alone() {
    assert_eq!(Scenario::new("passes", passes).run_on(&()), Ok(()));
    assert!(Scenario::new("panics", panics).run_on(&()).is_err());
}

#[test]
fn the_heartbeat_of_the_kits_counts_and_stops_when_told() {
    use ucdk_release_seams::{Heartbeat, Stop};
    let counting = Beats::counting();
    assert_eq!(counting.beat(), Ok(()));
    assert_eq!(counting.beat(), Ok(()));
    assert_eq!(counting.count(), 2);
    let stopping = Beats::stopping_at(2);
    assert_eq!(stopping.beat(), Ok(()));
    assert_eq!(stopping.beat(), Err(Stop));
    assert_eq!(stopping.beat(), Err(Stop));
}

#[test]
fn a_watcher_is_told_the_number_of_each_beat_in_order() {
    use ucdk_release_seams::{Heartbeat, Stop};
    let seen = RefCell::new(Vec::new());
    let watcher = |n: u32| seen.borrow_mut().push(n);
    let beats = Beats::stopping_at(3).watched_by(&watcher);
    assert_eq!(beats.beat(), Ok(()));
    assert_eq!(beats.beat(), Ok(()));
    assert_eq!(beats.beat(), Err(Stop));
    assert_eq!(*seen.borrow(), vec![1, 2, 3]);
}
