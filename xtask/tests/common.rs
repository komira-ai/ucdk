// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]
//! Helpers shared by the integration tests.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use xtask::cmd::{Cmd, Runner, System};

/// A fresh, empty directory under the build's scratch area.
pub fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

pub fn write(dir: &Path, rel: &str, text: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("create dirs");
    fs::write(path, text).expect("write file");
}

pub fn strings(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// Runs git in `dir` as a fixture author; returns its standard output.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let cmd = Cmd::new(
        "git",
        &[
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.org",
            ],
            args,
        ]
        .concat(),
        dir,
    );
    System.run(&cmd).unwrap_or_else(|e| panic!("{e}"))
}

pub type Hook = Box<dyn Fn(&Cmd)>;

/// A runner that records every command, fails the ones that contain a marker, and answers the
/// ones that contain another marker with fixed output.
#[derive(Default)]
pub struct Fake {
    pub log: RefCell<Vec<String>>,
    pub fail_on: Vec<&'static str>,
    pub answers: Vec<(&'static str, String)>,
    pub on_run: Option<Hook>,
}

impl Runner for Fake {
    fn run(&self, cmd: &Cmd) -> Result<String, String> {
        let line = cmd.to_string();
        self.log.borrow_mut().push(line.clone());
        if let Some(hook) = &self.on_run {
            hook(cmd);
        }
        if self.fail_on.iter().any(|m| line.contains(m)) {
            return Err(format!("{line} failed"));
        }
        let answer = self.answers.iter().find(|(m, _)| line.contains(m));
        Ok(answer.map(|(_, out)| out.clone()).unwrap_or_default())
    }
}

/// Marks every pinned tool as installed under `root`, so no download is attempted.
pub fn cache_tools(root: &Path) {
    let bin = xtask::fetch::bin_dir(root);
    fs::create_dir_all(&bin).expect("create bin dir");
    for tool in xtask::fetch::pinned() {
        fs::write(bin.join(format!("{}.sha256", tool.name)), &tool.sha256).expect("write marker");
    }
}
