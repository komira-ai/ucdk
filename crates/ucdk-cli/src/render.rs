// SPDX-License-Identifier: Apache-2.0
//! The text a command prints. The plan text is what a person reads before approving, and what
//! `apply` and `destroy` check the saved plan against.

use std::fmt::Write as _;

use ucdk_api::v1::ExitClass;
use ucdk_engine::{EngineError, Failure, Leak, ObjectStatus, Report, Standing, State};
use ucdk_plan::{Action, Plan, PlanHash, Retained};
use ucdk_provider::Status;

use crate::args::Mode;
use crate::world::ProviderName;

/// The first lines of a plan: what it is for, where, and the hash to pin. A saved plan must
/// start with exactly the header the approved hash gives.
pub fn header(provider: ProviderName, mode: Mode, hash: &PlanHash) -> String {
    format!(
        "ucdk plan\nprovider: {}\nmode: {}\nplan-hash: {hash}\n",
        provider.as_str(),
        mode.as_str()
    )
}

fn why(retained: &Retained) -> String {
    match retained {
        Retained::Keep => "it holds data".to_owned(),
        Retained::Undeletable(reason) => format!("it cannot be deleted: {reason}"),
        Retained::OwnerNotNamed => "the file no longer names its owner".to_owned(),
    }
}

/// The whole plan: the header, one line per step in the order they run, and a summary.
pub fn plan_text(provider: ProviderName, mode: Mode, plan: &Plan) -> String {
    let mut text = header(provider, mode, plan.hash());
    let mut changes = 0;
    for step in plan.steps() {
        let (word, note) = match &step.action {
            Action::Create => ("create", String::new()),
            Action::Update => ("update", String::new()),
            Action::Noop => ("ok", String::new()),
            Action::Converging => ("wait", " (still converging; nothing is written)".to_owned()),
            Action::KnownAfterApply => (
                "later",
                " (reads a value that is known only after apply)".to_owned(),
            ),
            Action::Delete { .. } => ("delete", String::new()),
            Action::Reap => ("reap", " (already gone; the record is dropped)".to_owned()),
            Action::Retain(r) => ("retain", format!(" ({})", why(r))),
        };
        if !matches!(
            step.action,
            Action::Noop | Action::Converging | Action::Retain(_)
        ) {
            changes += 1;
        }
        let _ = writeln!(
            text,
            "  {word:<7} {} [{}]{note}",
            step.path,
            step.kind.as_str()
        );
    }
    text.push_str(&if changes == 0 {
        "no changes\n".to_owned()
    } else {
        format!("{changes} to change\n")
    });
    text
}

/// What the ledger and the provider show, one line per object: the record's state, the live
/// object's status, and whether the two agree.
pub fn status_text(provider: ProviderName, objects: &[ObjectStatus]) -> String {
    let mut text = format!("ucdk status\nprovider: {}\n", provider.as_str());
    for object in objects {
        let record = object.record.map_or("none", |state| match state {
            State::Intent => "intent",
            State::Confirmed => "confirmed",
            State::Reaping => "reaping",
        });
        let live = object.live.map_or("absent", |status| match status {
            Status::Ready => "ready",
            Status::Converging => "converging",
            Status::Failed => "failed",
        });
        let note = match object.standing {
            Standing::Agrees => "agrees",
            Standing::Pending => "pending (a run began and did not finish)",
            Standing::Missing => "missing (recorded, and the object is gone)",
            Standing::NotApplied => "not applied",
            Standing::Unclaimed => "unclaimed (not ours, or ours with no record)",
        };
        let _ = writeln!(
            text,
            "  {} [{}] record: {record}, live: {live}, {note}",
            object.path,
            object.kind.as_str()
        );
    }
    let _ = writeln!(text, "{} objects", objects.len());
    text
}

fn leak_line(leak: &Leak) -> String {
    format!(
        "{} [{}] {:?} (stamped for {}, node {}; the ledger has no record of it)",
        leak.physical,
        leak.kind.as_str(),
        leak.name,
        leak.owner,
        leak.node
    )
}

/// The leaks a sweep found, one line per object, then a summary. Nothing is deleted: the
/// stamp says whose the object claims to be, and a person decides what to do with it.
pub fn leaks_text(provider: ProviderName, leaks: &[Leak]) -> String {
    let mut text = format!("ucdk leaks\nprovider: {}\n", provider.as_str());
    for leak in leaks {
        let _ = writeln!(text, "  leaked  {}", leak_line(leak));
    }
    let _ = writeln!(text, "{} leaks", leaks.len());
    text
}

/// What a run did, one line per object, then a summary.
pub fn report_text(report: &Report) -> String {
    let mut text = String::new();
    let lists = [
        ("created", &report.created),
        ("updated", &report.updated),
        ("resumed", &report.resumed),
        ("deleted", &report.deleted),
        ("reaped", &report.reaped),
    ];
    let mut changes = 0;
    for (word, paths) in lists {
        for path in paths {
            let _ = writeln!(text, "  {word:<8} {path}");
            changes += 1;
        }
    }
    for (path, retained) in &report.retained {
        let _ = writeln!(text, "  retained {path} ({})", why(retained));
    }
    for path in &report.converging {
        let _ = writeln!(
            text,
            "  waiting  {path} (still converging; its record is not confirmed until it is ready)"
        );
    }
    for path in &report.failed {
        let _ = writeln!(
            text,
            "  failed   {path} (still failed after the write; its record is not confirmed, run again to retry)"
        );
    }
    for (path, producer) in &report.blocked_by_failed {
        let _ = writeln!(
            text,
            "  blocked  {path} (its producer {producer} is still failed)"
        );
    }
    for leak in &report.leaks {
        let _ = writeln!(text, "  leaked   {}", leak_line(leak));
    }
    if let Some(error) = &report.leak_sweep_failed {
        let _ = writeln!(text, "  leak sweep failed: {error}");
    }
    text.push_str(&if changes == 0 {
        "no changes\n".to_owned()
    } else {
        format!("{changes} changed\n")
    });
    text
}

/// The message for standard error of a run that stopped, with the advice its exit class gives:
/// nothing was changed, so the run is safe to repeat; or something may have been, so the world
/// is read (planned) again before running again.
pub fn failure(failure: &Failure, class: ExitClass) -> String {
    let advice = if matches!(failure.error, EngineError::LeaseUnconfirmed { .. }) {
        "the ledger lease may still be held; run again with the same --holder, or wait for it to lapse"
    } else if class == ExitClass::FailedNoEffect {
        "nothing was changed; it is safe to run again"
    } else {
        "the run changed things before it stopped; plan again before running again"
    };
    let place = failure
        .at
        .as_ref()
        .map_or_else(String::new, |at| format!(" at {at}"));
    format!("failed{place}: {}\n{advice}\n", failure.error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ucdk_provider::{Kind, NodePath, PhysicalId, ProviderError};

    /// Proves: a run that may still hold the ledger lease is not told it is "safe to run
    /// again" whatever its exit class, and is told how to proceed.
    #[test]
    fn a_failure_that_may_hold_the_lease_gives_its_own_advice() {
        let failure_of = |class| {
            failure(
                &Failure {
                    report: Report::default(),
                    at: None,
                    error: EngineError::LeaseUnconfirmed {
                        why: "down".to_owned(),
                        run: None,
                    },
                },
                class,
            )
        };
        for class in [ExitClass::FailedNoEffect, ExitClass::FailedChanged] {
            let text = failure_of(class);
            assert!(
                text.ends_with("run again with the same --holder, or wait for it to lapse\n"),
                "{text}"
            );
            assert!(!text.contains("safe to run again"), "{text}");
        }
    }

    /// Proves: a report that carries a leak and a sweep that could not read prints both, under
    /// the run's lines and above its summary. Catches a failed sweep that leaves no trace in the
    /// text (a reader would take it for "no leaks"), which no command-level test can arm because
    /// the sweep is the last read of an apply.
    #[test]
    fn a_report_prints_its_leaks_and_a_failed_sweep() {
        let path = NodePath::new("ghost/run").expect("a path");
        let report = Report {
            leaks: vec![Leak {
                kind: Kind::service(),
                physical: PhysicalId::new("svc-1"),
                name: "ghost-run".to_owned(),
                owner: path.owner(),
                node: path.key(),
            }],
            leak_sweep_failed: Some(EngineError::Provider(ProviderError::Transient(
                "down".to_owned(),
            ))),
            ..Report::default()
        };
        let text = report_text(&report);
        let want_leak = format!(
            "  leaked   svc-1 [service] \"ghost-run\" (stamped for ghost, node {}; the ledger has no record of it)\n",
            path.key()
        );
        assert!(text.starts_with(&want_leak), "{text}");
        let rest = &text[want_leak.len()..];
        assert!(rest.starts_with("  leak sweep failed: "), "{text}");
        assert!(rest.contains("down"), "{text}");
        assert!(rest.ends_with("\nno changes\n"), "{text}");
    }
}
