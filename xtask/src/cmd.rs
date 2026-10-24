// SPDX-License-Identifier: Apache-2.0
//! Running other programs. Checks build a `Cmd` and hand it to a `Runner`, so tests can
//! record the commands or fake their results.

use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A program to run, with the directory to run it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
    pub dir: PathBuf,
    /// A directory put in front of `PATH` for this command only (where fetched tools live).
    pub path_prepend: Option<PathBuf>,
}

impl Cmd {
    pub fn new(program: &str, args: &[&str], dir: &Path) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            dir: dir.to_owned(),
            path_prepend: None,
        }
    }

    #[must_use]
    pub fn with_path(mut self, dir: &Path) -> Self {
        self.path_prepend = Some(dir.to_owned());
        self
    }
}

impl fmt::Display for Cmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let words =
            std::iter::once(self.program.as_str()).chain(self.args.iter().map(String::as_str));
        write!(f, "{}", words.collect::<Vec<_>>().join(" "))
    }
}

/// Runs a command: its standard output on success, its error text on failure.
pub trait Runner {
    fn run(&self, cmd: &Cmd) -> Result<String, String>;
}

/// The real thing.
pub struct System;

impl Runner for System {
    fn run(&self, cmd: &Cmd) -> Result<String, String> {
        let mut command = Command::new(&cmd.program);
        command.args(&cmd.args).current_dir(&cmd.dir);
        if let Some(prepend) = &cmd.path_prepend {
            let mut paths = vec![prepend.clone()];
            paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
            command.env("PATH", env::join_paths(paths).unwrap_or_default());
        }
        let out = command.output().map_err(|e| format!("{cmd}: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if out.status.success() {
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            Err(format!("{cmd}: {}\n{stdout}{stderr}", out.status))
        }
    }
}
