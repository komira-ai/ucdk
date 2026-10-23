// SPDX-License-Identifier: Apache-2.0
//! The `ucdk` command line. Pre-release: it answers `--version` and nothing else.

use std::io::{self, Write};
use std::process::ExitCode;

/// Runs the command line over `args` (without the program name); returns the exit code.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    match args {
        [flag] if flag == "--version" => {
            let _ = writeln!(out, "ucdk {}", env!("CARGO_PKG_VERSION"));
            0
        }
        _ => {
            let _ = writeln!(err, "usage: ucdk --version");
            2
        }
    }
}

/// The process entry point: the real arguments, standard output and standard error.
pub fn main_entry() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    ExitCode::from(run(&args, &mut io::stdout(), &mut io::stderr()))
}
