// SPDX-License-Identifier: Apache-2.0
//! The `ucdk` command line. Pre-release: it answers `--version` and nothing else.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--version" => {
            println!("ucdk {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: ucdk --version");
            ExitCode::from(2)
        }
    }
}
