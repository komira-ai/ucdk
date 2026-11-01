// SPDX-License-Identifier: Apache-2.0
//! The class a run ends in, from the verdict that stopped it, the reason it carries and whether
//! the run recorded a landed or staged coordinate.

use ucdk_api::exit_class;
use ucdk_api::v1::ExitClass;
use ucdk_release_proto::v1::{Reason, Verdict};

fn verdicts() -> Vec<Verdict> {
    (0..64).filter_map(|n| Verdict::try_from(n).ok()).collect()
}

fn reasons() -> Vec<Reason> {
    (0..256).filter_map(|n| Reason::try_from(n).ok()).collect()
}

fn is_done(verdict: Verdict) -> bool {
    matches!(
        verdict,
        Verdict::Succeeded | Verdict::Noop | Verdict::Superseded
    )
}

fn is_lease_reason(reason: Reason) -> bool {
    matches!(reason, Reason::LeaseLost | Reason::LeaseUnconfirmed)
}

#[test]
fn each_verdict_has_the_class_the_exit_table_gives_it() {
    use ExitClass::{
        Done, FailedChanged, FailedNoEffect, Indeterminate, Refused, ValidationFailed,
    };
    // (verdict, class when nothing landed, class when something landed)
    let table = [
        (Verdict::Succeeded, Done, Done),
        (Verdict::Noop, Done, Done),
        (Verdict::Superseded, Done, Done),
        (Verdict::Refused, Refused, FailedChanged),
        (Verdict::Failed, FailedNoEffect, FailedChanged),
        (Verdict::Indeterminate, Indeterminate, FailedChanged),
        (Verdict::Partial, FailedChanged, FailedChanged),
        (Verdict::Interrupted, FailedChanged, FailedChanged),
        (
            Verdict::ValidationFailed,
            ValidationFailed,
            ValidationFailed,
        ),
        // A verdict nobody set is a bug of the producer; it never claims that nothing changed.
        (Verdict::Unspecified, FailedChanged, FailedChanged),
    ];
    for (verdict, nothing_landed, landed) in table {
        let reason = Reason::Unspecified;
        assert_eq!(
            exit_class(verdict, reason, false),
            nothing_landed,
            "{verdict:?}"
        );
        assert_eq!(exit_class(verdict, reason, true), landed, "{verdict:?}");
    }
}

#[test]
fn the_table_names_every_verdict_the_wire_defines() {
    assert_eq!(verdicts().len(), 10, "{:?}", verdicts());
}

#[test]
fn every_verdict_has_a_class() {
    // Every verdict with every reason, landed or not, ends in one of the six classes a run can
    // have: the function is total and never answers a class that has no producer.
    let allowed = [
        ExitClass::Done,
        ExitClass::Refused,
        ExitClass::FailedNoEffect,
        ExitClass::Indeterminate,
        ExitClass::FailedChanged,
        ExitClass::ValidationFailed,
    ];
    for verdict in verdicts() {
        for reason in reasons() {
            for landed in [false, true] {
                let class = exit_class(verdict, reason, landed);
                assert!(
                    allowed.contains(&class),
                    "{verdict:?} {reason:?} {landed}: {class:?}"
                );
            }
        }
    }
}

#[test]
fn no_effect_classes_never_with_a_landed_record() {
    for verdict in verdicts() {
        for reason in reasons() {
            let class = exit_class(verdict, reason, true);
            assert!(
                !class.promises_no_effect(),
                "{verdict:?} {reason:?}: {class:?}"
            );
        }
    }
}

#[test]
fn indeterminate_with_a_landed_record_is_class_6() {
    for reason in reasons() {
        let class = exit_class(Verdict::Indeterminate, reason, true);
        assert_eq!(class, ExitClass::FailedChanged, "{reason:?}");
        assert_eq!(class.code(), 6);
    }
}

#[test]
fn a_lost_or_unconfirmed_lease_is_class_6_whatever_the_verdict_and_whether_or_not_it_landed() {
    for verdict in verdicts().into_iter().filter(|v| !is_done(*v)) {
        for reason in reasons().into_iter().filter(|r| is_lease_reason(*r)) {
            for landed in [false, true] {
                assert_eq!(
                    exit_class(verdict, reason, landed),
                    ExitClass::FailedChanged,
                    "{verdict:?} {reason:?} {landed}"
                );
            }
        }
    }
}

#[test]
fn a_done_verdict_is_done_whatever_the_reason() {
    for verdict in verdicts().into_iter().filter(|v| is_done(*v)) {
        for reason in reasons() {
            for landed in [false, true] {
                assert_eq!(
                    exit_class(verdict, reason, landed),
                    ExitClass::Done,
                    "{verdict:?} {reason:?} {landed}"
                );
            }
        }
    }
}

#[test]
fn any_other_reason_leaves_the_class_to_the_verdict_and_landed() {
    for verdict in verdicts() {
        for reason in reasons().into_iter().filter(|r| !is_lease_reason(*r)) {
            for landed in [false, true] {
                assert_eq!(
                    exit_class(verdict, reason, landed),
                    exit_class(verdict, Reason::Unspecified, landed),
                    "{verdict:?} {reason:?} {landed}"
                );
            }
        }
    }
}

#[test]
fn usage_and_leaks_have_no_producer() {
    for verdict in verdicts() {
        for reason in reasons() {
            for landed in [false, true] {
                let class = exit_class(verdict, reason, landed);
                assert!(
                    class != ExitClass::Usage && class != ExitClass::Leaks,
                    "{verdict:?} {reason:?} {landed}: {class:?}"
                );
            }
        }
    }
}
