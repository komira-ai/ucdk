// SPDX-License-Identifier: Apache-2.0
//! The `ucdk` command line as a library: `run(args, &mut World)` returns the exit code, so a
//! test drives the same code the binary does, in-process.
//!
//! Verbs: `plan`, `apply` and `destroy`, against the `fake` or `fake-limited` provider. All
//! input is flags: `--input` is a binary `ucdk.resource.v1.ResourceList`.
//!
//! Exit codes:
//!
//! | code | meaning |
//! |---|---|
//! | 0 | done, or planned; including "no changes", and including a run that left an object still converging or still failed (a note says so) |
//! | 2 | usage: a bad flag, or a file that cannot be read, written or decoded |
//! | 3 | refused, nothing was changed: findings, a kind the provider lacks, an object that is not ours, a plan that is not the approved one |
//! | 4 | failed with no effect: safe to run again |
//! | 6 | failed after changing something: plan again before running again |

mod args;
mod exec;
mod render;
mod world;

use std::io::Write as _;
use std::process::ExitCode;

pub use args::USAGE;
pub use world::{ProviderName, World};

/// Runs the command line over `args` (without the program name) in `world`; the output is
/// appended to `world.out` and `world.err`, and the exit code is returned.
pub fn run(args: &[String], world: &mut World) -> u8 {
    exec::run(args, world)
}

/// The process entry point: the real arguments, a new world, and the output written to the
/// standard streams.
pub fn main_entry() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut world = World::new();
    let code = run(&args, &mut world);
    let _ = std::io::stdout().write_all(world.out.as_bytes());
    let _ = std::io::stderr().write_all(world.err.as_bytes());
    ExitCode::from(code)
}
