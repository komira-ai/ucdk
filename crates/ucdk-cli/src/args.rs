// SPDX-License-Identifier: Apache-2.0
//! Reading the command line: a verb and its flags, nothing positional.

use std::collections::BTreeMap;

use ucdk_plan::PlanHash;

use crate::world::ProviderName;

pub const USAGE: &str = "\
usage: ucdk --version
       ucdk plan --provider=<fake|fake-limited> --input=<resources.pb> [--mode=<apply|destroy>] [--out=<plan file>]
       ucdk status --provider=<fake|fake-limited> --input=<resources.pb>
       ucdk leaks --provider=<fake|fake-limited>
       ucdk apply --provider=<fake|fake-limited> --input=<resources.pb> --plan=<plan file> --expect-plan-hash=<hash>
       ucdk destroy --provider=<fake|fake-limited> --input=<resources.pb> --plan=<plan file> --expect-plan-hash=<hash>
";

/// A saved plan and the hash the approver pinned for it.
#[derive(Debug)]
pub struct Approval {
    pub plan: String,
    pub hash: PlanHash,
}

#[derive(Debug)]
pub enum Verb {
    /// Show what a run would do; `out` is where to save the plan text.
    Plan {
        out: Option<String>,
    },
    Apply(Approval),
    Destroy(Approval),
    /// Show what the ledger records and the provider holds; changes nothing.
    Status,
    /// Show the cell's stamped objects the ledger does not record; changes nothing, and ends
    /// with exit 8 when it finds some.
    Leaks,
}

/// What a plan is made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Apply,
    Destroy,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Apply => "apply",
            Self::Destroy => "destroy",
        }
    }
}

#[derive(Debug)]
pub struct Invocation {
    pub verb: Verb,
    pub provider: ProviderName,
    pub mode: Mode,
    /// The resource list file; empty for `leaks`, which reads no file.
    pub input: String,
}

#[derive(Debug)]
pub enum Command {
    Version,
    Run(Invocation),
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

/// The flags of a verb: those it needs, then those it may take. Any other flag is an error.
fn split_flags(
    mut flags: BTreeMap<String, String>,
    needed: &[&'static str],
    optional: &[&'static str],
) -> Result<BTreeMap<&'static str, String>, String> {
    let mut values = BTreeMap::new();
    for name in needed {
        let value = flags
            .remove(*name)
            .ok_or_else(|| format!("`--{name}` is needed"))?;
        values.insert(*name, value);
    }
    for name in optional {
        values.extend(flags.remove(*name).map(|value| (*name, value)));
    }
    match flags.keys().next() {
        Some(name) => Err(format!("`--{name}` is not a flag of this command")),
        None => Ok(values),
    }
}

fn approval(values: &BTreeMap<&'static str, String>) -> Result<Approval, String> {
    let text = &values["expect-plan-hash"];
    Ok(Approval {
        plan: values["plan"].clone(),
        hash: PlanHash::from_hex(text).ok_or_else(|| format!("`{text}` is not a plan hash"))?,
    })
}

/// Reads the arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Command, String> {
    let (verb, rest) = match args.split_first() {
        Some((first, [])) if first == "--version" => return Ok(Command::Version),
        Some((first, rest)) => (first.as_str(), rest),
        None => return Err("a command is needed".to_owned()),
    };
    let flags = read_flags(rest)?;
    let both: &[&str] = &["provider", "input", "plan", "expect-plan-hash"];
    let (values, mode) = match verb {
        "plan" => {
            let values = split_flags(flags, &["provider", "input"], &["mode", "out"])?;
            let mode = match values.get("mode").map(String::as_str) {
                Some("destroy") => Mode::Destroy,
                Some("apply") | None => Mode::Apply,
                Some(other) => return Err(format!("`{other}` is not a mode (apply, destroy)")),
            };
            (values, mode)
        }
        "status" => (
            split_flags(flags, &["provider", "input"], &[])?,
            Mode::Apply,
        ),
        "leaks" => (split_flags(flags, &["provider"], &[])?, Mode::Apply),
        "apply" => (split_flags(flags, both, &[])?, Mode::Apply),
        "destroy" => (split_flags(flags, both, &[])?, Mode::Destroy),
        other => return Err(format!("`{other}` is not a command")),
    };
    let verb = match verb {
        "plan" => Verb::Plan {
            out: values.get("out").cloned(),
        },
        "status" => Verb::Status,
        "leaks" => Verb::Leaks,
        "apply" => Verb::Apply(approval(&values)?),
        _ => Verb::Destroy(approval(&values)?),
    };
    let provider = &values["provider"];
    Ok(Command::Run(Invocation {
        verb,
        provider: ProviderName::parse(provider)
            .ok_or_else(|| format!("`{provider}` is not a provider (fake, fake-limited)"))?,
        mode,
        input: values.get("input").cloned().unwrap_or_default(),
    }))
}
