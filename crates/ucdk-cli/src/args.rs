// SPDX-License-Identifier: Apache-2.0
//! Reading the command line: a verb of one or two words and its flags, nothing positional.

use std::collections::BTreeMap;

pub const USAGE: &str = "\
usage: ucdk --version
       ucdk ledger show --ledger=<ledger file> [--holder=<name>]
";

#[derive(Debug)]
pub enum Command {
    Version,
    /// Show a ledger file; the lease is reported `yours` when it names `holder`.
    LedgerShow {
        ledger: String,
        holder: Option<String>,
    },
}

/// `--name=value` or `--name value`, each name at most once.
fn read_flags(args: &[String]) -> Result<BTreeMap<String, String>, String> {
    let mut flags = BTreeMap::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let Some(body) = arg.strip_prefix("--") else {
            return Err(format!(
                "`{arg}` is not a flag (there are no positional arguments)"
            ));
        };
        let (name, value) = match body.split_once('=') {
            Some((name, value)) => (name, value.to_owned()),
            None => (
                body,
                rest.next()
                    .ok_or_else(|| format!("`--{body}` needs a value"))?
                    .clone(),
            ),
        };
        if flags.insert(name.to_owned(), value).is_some() {
            return Err(format!("`--{name}` is given twice"));
        }
    }
    Ok(flags)
}

/// Reads the arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Command, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] => Err("a command is needed".to_owned()),
        ["--version"] => Ok(Command::Version),
        ["--version", extra, ..] => Err(format!(
            "`--version` takes no argument, but `{extra}` is given"
        )),
        ["ledger", "show", ..] => {
            let mut flags = read_flags(&args[2..])?;
            let ledger = flags
                .remove("ledger")
                .ok_or_else(|| "`--ledger` is needed".to_owned())?;
            let holder = flags.remove("holder");
            match flags.keys().next() {
                Some(name) => Err(format!("`--{name}` is not a flag of this command")),
                None => Ok(Command::LedgerShow { ledger, holder }),
            }
        }
        ["ledger"] => Err("`ledger` needs a command (show)".to_owned()),
        ["ledger", other, ..] => Err(format!("`ledger {other}` is not a command")),
        [first, ..] => Err(format!("`{first}` is not a command")),
    }
}
