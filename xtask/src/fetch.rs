// SPDX-License-Identifier: Apache-2.0
//! The external tools the checks need, each downloaded from a fixed URL and verified against a
//! SHA-256 pin before it is unpacked. Nothing is compiled: a tool is a release binary. The
//! cache lives under `target/` and is keyed by the pin, so changing a pin fetches again.
//! The pins are for Linux on x86-64.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::cmd::{Cmd, Runner};
use crate::{Finding, io_err};

/// One release archive and the executable inside it.
#[derive(Debug, Clone)]
pub struct Tool {
    /// The executable's name once installed (`cargo-deny`).
    pub name: String,
    pub url: String,
    /// Hex SHA-256 of the archive.
    pub sha256: String,
    /// The path of the executable inside the archive.
    pub member: String,
}

/// The pinned tools.
pub fn pinned() -> Vec<Tool> {
    let tool = |name: &str, url: &str, sha256: &str, member: &str| Tool {
        name: name.to_owned(),
        url: url.to_owned(),
        sha256: sha256.to_owned(),
        member: member.to_owned(),
    };
    vec![
        tool(
            "cargo-deny",
            "https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-x86_64-unknown-linux-musl.tar.gz",
            "9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f",
            "cargo-deny-0.20.2-x86_64-unknown-linux-musl/cargo-deny",
        ),
        tool(
            "cargo-llvm-cov",
            "https://github.com/taiki-e/cargo-llvm-cov/releases/download/v0.9.1/cargo-llvm-cov-x86_64-unknown-linux-gnu.tar.gz",
            "b3f68e625481fed9b16444174f3fa5ebcdbde4a1878803a35eabe2dcefcdc41a",
            "cargo-llvm-cov",
        ),
    ]
}

/// Where the installed tools live: the directory the checks put on `PATH`.
pub fn bin_dir(root: &Path) -> PathBuf {
    root.join("target").join("xtask-tools").join("bin")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Installs `tool` into `bin` unless the pinned version is already there.
pub fn ensure(tool: &Tool, bin: &Path, runner: &dyn Runner) -> Result<PathBuf, String> {
    let dest = bin.join(&tool.name);
    let marker = bin.join(format!("{}.sha256", tool.name));
    if matches!(fs::read_to_string(&marker), Ok(pin) if pin == tool.sha256) {
        return Ok(dest);
    }
    let work = bin.join(format!("{}.work", tool.name));
    fs::create_dir_all(&work).map_err(io_err)?;
    let archive = work.join("archive.tar.gz");
    let unpacked = work.join("unpacked");
    fs::create_dir_all(&unpacked).map_err(io_err)?;
    let (archive_arg, unpacked_arg) = (archive.to_string_lossy(), unpacked.to_string_lossy());
    let curl = [
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https,file",
        "--proto-redir",
        "=https",
        "--output",
        &archive_arg,
        &tool.url,
    ];
    runner.run(&Cmd::new("curl", &curl, bin))?;
    let got = sha256_hex(&fs::read(&archive).map_err(io_err)?);
    if got != tool.sha256 {
        return Err(format!(
            "{}: the download has SHA-256 {got}, the pin is {}",
            tool.name, tool.sha256
        ));
    }
    runner.run(&Cmd::new(
        "tar",
        &["-xzf", &archive_arg, "-C", &unpacked_arg],
        bin,
    ))?;
    fs::copy(unpacked.join(&tool.member), &dest).map_err(io_err)?;
    fs::write(&marker, &tool.sha256).map_err(io_err)?;
    fs::remove_dir_all(&work).map_err(io_err)?;
    Ok(dest)
}

/// Installs every pinned tool under `root`; each failure is a finding.
pub fn ensure_all(root: &Path, runner: &dyn Runner) -> Vec<Finding> {
    let bin = bin_dir(root);
    let mut out = Vec::new();
    if let Err(e) = fs::create_dir_all(&bin) {
        out.push(Finding::new("fetch", "tools", e.to_string()));
        return out;
    }
    for tool in pinned() {
        if let Err(e) = ensure(&tool, &bin, runner) {
            out.push(Finding::new("fetch", &tool.name, e));
        }
    }
    out
}
