// SPDX-License-Identifier: Apache-2.0
//! Running a command: parse the arguments, ask `ucdk_api::ledger_show` for the answer, print it.

use std::path::Path;
use std::sync::Arc;

use ucdk_api::v1::{ExitClass, outcome};
use ucdk_api::{LedgerSource, ledger_show};
use ucdk_ledger::default_holder;

use crate::args::{Command, USAGE, parse};
use crate::render::ledger_text;
use crate::world::World;

pub fn run(args: &[String], world: &mut World) -> u8 {
    let class = match parse(args) {
        Err(message) => {
            world.err.push_str(&format!("ucdk: {message}\n{USAGE}"));
            ExitClass::Usage
        }
        Ok(Command::Version) => {
            world
                .out
                .push_str(concat!("ucdk ", env!("CARGO_PKG_VERSION"), "\n"));
            ExitClass::Done
        }
        Ok(Command::LedgerShow { ledger, holder }) => show(&ledger, holder, world),
    };
    class.code()
}

/// Shows the ledger file at `path`, reading it as `holder` (a lease is `yours` when it names
/// that holder; with none given, the default holder, which no lease names).
fn show(path: &str, holder: Option<String>, world: &mut World) -> ExitClass {
    let holder = holder.unwrap_or_else(default_holder);
    let answer = ledger_show(
        &LedgerSource::File(Path::new(path)),
        &holder,
        Arc::clone(&world.clock),
    );
    let class = ExitClass::try_from(answer.exit).expect("ledger_show sets a class it defines");
    match answer.result.expect("ledger_show always sets a result") {
        outcome::Result::LedgerShow(response) => world.out.push_str(&ledger_text(&response)),
        outcome::Result::Error(error) | outcome::Result::Invalid(error) => {
            world.err.push_str(&format!("ucdk: {}\n", error.message));
        }
    }
    class
}
