// SPDX-License-Identifier: Apache-2.0
//! The `ucdk` command line as a library: `run(args, &mut World)` returns the exit code, so a
//! test drives the same code the binary does, in-process.
//!
//! The one verb is `ucdk ledger show --ledger <file> [--holder <name>]`: it prints one ledger
//! file, one fact per line (see [`ledger_text`]), and takes no lease and changes no file. All
//! input is flags, and there are no positional arguments.
//!
//! Exit codes (the table is `ucdk_api::v1::ExitClass::code`):
//!
//! | code | meaning |
//! |---|---|
//! | 0 | done |
//! | 2 | usage: a missing, unknown or repeated flag, an unknown command, or a positional argument |
//! | 4 | failed with no effect: the ledger file is missing, corrupt, of a newer major, or cannot be read |

mod args;
mod exec;
mod render;
mod world;

use std::io::Write as _;
use std::process::ExitCode;

pub use args::USAGE;
pub use render::ledger_text;
pub use world::World;

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
