// SPDX-License-Identifier: Apache-2.0
//! The text a command prints. The plan text is what a person reads before approving, and what
//! `apply` and `destroy` check the saved plan against.

use std::fmt::Write as _;

use ucdk_engine::{EngineError, Failure, Report};
use ucdk_plan::{Action, Plan, PlanHash, Retained};
use ucdk_provider::ProviderError;

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
    text.push_str(&if changes == 0 {
        "no changes\n".to_owned()
    } else {
        format!("{changes} changed\n")
    });
    text
}

/// The exit code of a run that stopped, and the message for standard error. Code 4: nothing
/// was changed, so the run is safe to repeat. Code 6: something may have been, so the world
/// is read (planned) again before running again.
pub fn failure(failure: &Failure) -> (u8, String) {
    let safe_error = matches!(
        &failure.error,
        EngineError::Provider(
            ProviderError::Refused(_) | ProviderError::Permanent(_) | ProviderError::Transient(_)
        )
    );
    let nothing_done = failure.report == Report::default();
    let (code, advice) = if nothing_done && (failure.at.is_none() || safe_error) {
        (4, "nothing was changed; it is safe to run again")
    } else {
        (
            6,
            "the run changed things before it stopped; plan again before running again",
        )
    };
    let place = failure
        .at
        .as_ref()
        .map_or_else(String::new, |at| format!(" at {at}"));
    (
        code,
        format!("failed{place}: {}\n{advice}\n", failure.error),
    )
}
