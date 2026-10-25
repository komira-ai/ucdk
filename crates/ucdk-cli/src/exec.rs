// SPDX-License-Identifier: Apache-2.0
//! Running a command: read the input, plan against the world, and for `apply` and `destroy`
//! hand the approved plan to the engine; `status` only reads.

use std::collections::BTreeSet;
use std::fs;

use prost::Message as _;
use ucdk_api::exit_class;
use ucdk_api::v1::ExitClass;
use ucdk_engine::{Engine, Mode as EngineMode, Outcome, Report, Request};
use ucdk_plan::{Lowered, lower};
use ucdk_proto::v1::ResourceList;

use crate::args::{Approval, Command, Invocation, Mode, USAGE, Verb, parse};
use crate::render::{failure, header, plan_text, report_text, status_text};
use crate::world::World;

/// A command that stops: its exit code and what to print to standard error.
struct Exit {
    code: ExitClass,
    message: String,
}

fn usage(message: String) -> Exit {
    Exit {
        code: ExitClass::Usage,
        message,
    }
}

fn refused(message: String) -> Exit {
    Exit {
        code: ExitClass::Refused,
        message,
    }
}

pub fn run(args: &[String], world: &mut World) -> u8 {
    let result = match parse(args) {
        Err(message) => Err(usage(format!("{message}\n{USAGE}"))),
        Ok(Command::Version) => {
            world
                .out
                .push_str(concat!("ucdk ", env!("CARGO_PKG_VERSION"), "\n"));
            Ok(ExitClass::Done)
        }
        Ok(Command::Run(invocation)) => execute(&invocation, world),
    };
    result
        .unwrap_or_else(|exit| {
            world
                .err
                .push_str(&format!("ucdk: {}\n", exit.message.trim_end()));
            exit.code
        })
        .code()
}

fn read_input(path: &str) -> Result<Lowered, Exit> {
    let bytes = fs::read(path).map_err(|e| usage(format!("cannot read {path}: {e}")))?;
    let list = ResourceList::decode(bytes.as_slice())
        .map_err(|e| usage(format!("{path} is not a binary resource list: {e}")))?;
    lower(&list).map_err(|findings| {
        let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
        refused(format!(
            "the resource list is refused:\n  {}",
            lines.join("\n  ")
        ))
    })
}

/// A saved plan is the plan text a `plan` command printed. It must be the plan for this
/// provider and mode with exactly the hash the approver pinned.
fn check_saved(invocation: &Invocation, approval: &Approval) -> Result<(), Exit> {
    let path = &approval.plan;
    let saved = fs::read_to_string(path).map_err(|e| usage(format!("cannot read {path}: {e}")))?;
    let wanted = header(invocation.provider, invocation.mode, &approval.hash);
    if saved.starts_with(&wanted) {
        Ok(())
    } else {
        Err(refused(format!(
            "{path} is not the saved {} plan {} for {}",
            invocation.mode.as_str(),
            approval.hash,
            invocation.provider.as_str()
        )))
    }
}

/// One line for each wanted node whose kind the provider does not host.
fn unhosted(invocation: &Invocation, desired: &Lowered, world: &mut World) -> BTreeSet<String> {
    let cloud = world.cloud(invocation.provider);
    desired
        .nodes()
        .iter()
        .filter(|n| n.wanted && !cloud.hosts(n.node.kind()))
        .map(|n| {
            format!(
                "refused: kind {} is not hosted by {} ({})",
                n.node.kind().as_str(),
                invocation.provider.as_str(),
                n.node.path()
            )
        })
        .collect()
}

fn execute(invocation: &Invocation, world: &mut World) -> Result<ExitClass, Exit> {
    let desired = read_input(&invocation.input)?;
    if let Verb::Apply(approval) | Verb::Destroy(approval) = &invocation.verb {
        check_saved(invocation, approval)?;
    }
    if invocation.mode == Mode::Apply {
        let lines = unhosted(invocation, &desired, world);
        if !lines.is_empty() {
            let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
            // Only a plan, a status or an apply gets here: a destroy plans against no wanted node.
            let verb = match invocation.verb {
                Verb::Plan { .. } => "plan",
                Verb::Status => "status",
                _ => "apply",
            };
            world.out.push_str(&format!(
                "ucdk {verb}\nprovider: {}\nmode: {}\n{text}",
                invocation.provider.as_str(),
                invocation.mode.as_str(),
            ));
            return Ok(ExitClass::Refused);
        }
    }

    let ledger = world.ledger.clone();
    let cell = world.cell.clone();
    let mut engine = Engine::new(world.cloud(invocation.provider), &ledger, cell);
    if matches!(invocation.verb, Verb::Status) {
        let outcome = engine.handle(Request::Status { desired });
        return finish(outcome, invocation, world);
    }
    let mode = match invocation.mode {
        Mode::Apply => EngineMode::Apply,
        Mode::Destroy => EngineMode::Destroy,
    };
    let planned = engine.handle(Request::Plan {
        desired: desired.clone(),
        mode,
    });
    let outcome = match (&invocation.verb, planned) {
        (Verb::Apply(approval), Outcome::Planned(plan)) => engine.handle(Request::Apply {
            desired,
            plan,
            plan_hash: approval.hash.clone(),
            rollback: false,
        }),
        (Verb::Destroy(approval), Outcome::Planned(plan)) => engine.handle(Request::Destroy {
            desired,
            plan,
            plan_hash: approval.hash.clone(),
        }),
        (_, other) => other,
    };
    finish(outcome, invocation, world)
}

/// Prints what the command ended with and gives its exit class.
fn finish(outcome: Outcome, invocation: &Invocation, world: &mut World) -> Result<ExitClass, Exit> {
    let class = exit_class(&outcome);
    match outcome {
        Outcome::Planned(plan) => {
            let text = plan_text(invocation.provider, invocation.mode, &plan);
            if let Verb::Plan { out: Some(path) } = &invocation.verb {
                fs::write(path, &text).map_err(|e| usage(format!("cannot write {path}: {e}")))?;
            }
            world.out.push_str(&text);
            Ok(class)
        }
        Outcome::Done(report) => {
            world.out.push_str(&report_text(&report));
            Ok(class)
        }
        Outcome::Status(objects) => {
            world
                .out
                .push_str(&status_text(invocation.provider, &objects));
            Ok(class)
        }
        Outcome::Refused(why) => Err(Exit {
            code: class,
            message: why.to_string(),
        }),
        Outcome::Failed(stopped) => {
            if stopped.report != Report::default() {
                world.out.push_str(&report_text(&stopped.report));
            }
            Err(Exit {
                code: class,
                message: failure(&stopped, class),
            })
        }
    }
}
