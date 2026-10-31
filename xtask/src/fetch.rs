// SPDX-License-Identifier: Apache-2.0
//! The external tools the checks need, each downloaded from a fixed URL and verified against a
//! SHA-256 pin before it is unpacked. Nothing is compiled: a tool is a release binary. The
//! cache lives under `target/` and is keyed by the pin, so changing a pin fetches again.
//! The pins are for Linux on x86-64.
//!
//! A download is one of three kinds, told by the end of its URL: a `.tar.gz` (the executable is
//! the `member` of the archive), a `.zst` (the executable, compressed) or anything else (the
//! executable itself).

use std::fs;
use std::os::unix::fs::PermissionsExt;
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

/// The names of the pinned tools.
pub const DENY: &str = "cargo-deny";
pub const LLVM_COV: &str = "cargo-llvm-cov";
/// The Node the TypeScript SDK's tests run on. `node:ffi`, which binds the C library without an
/// addon or a package, is in Node 26; the version is pinned so that the SDK's tests do not move
/// with the machine's Node.
pub const NODE: &str = "node";
/// The build tools that the confirmation job runs the adapters against.
pub const BUCK2: &str = "buck2";
pub const BAZEL: &str = "bazel";

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
            DENY,
            "https://github.com/EmbarkStudios/cargo-deny/releases/download/0.20.2/cargo-deny-0.20.2-x86_64-unknown-linux-musl.tar.gz",
            "9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f",
            "cargo-deny-0.20.2-x86_64-unknown-linux-musl/cargo-deny",
        ),
        tool(
            LLVM_COV,
            "https://github.com/taiki-e/cargo-llvm-cov/releases/download/v0.9.1/cargo-llvm-cov-x86_64-unknown-linux-gnu.tar.gz",
            "b3f68e625481fed9b16444174f3fa5ebcdbde4a1878803a35eabe2dcefcdc41a",
            "cargo-llvm-cov",
        ),
        tool(
            NODE,
            "https://nodejs.org/dist/v26.12.1/node-v26.12.1-linux-x64.tar.gz",
            "0fd1eb7770598f52a21995edf6ed35fc690400fa437c2386af329519e9143132",
            "node-v26.12.1-linux-x64/bin/node",
        ),
    ]
}

/// The build tools of the confirmation job, which only that job installs: the releases of
/// Buck2 (a compressed executable) and Bazel (the executable itself).
pub fn confirmation_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: BUCK2.to_owned(),
            url: "https://github.com/meta/buck2/releases/download/2026-10-15/buck2-x86_64-unknown-linux-gnu.zst".to_owned(),
            sha256: "f0f21a31ed2e2428b6a0b4722a82372861051002d4c0cc95db7906e33119e3a3".to_owned(),
            member: String::new(),
        },
        Tool {
            name: BAZEL.to_owned(),
            url: "https://github.com/bazelbuild/bazel/releases/download/9.3.1/bazel-9.3.1-linux-x86_64".to_owned(),
            sha256: "7f775b54bbec487efaf0f49ccda1d2ed02cd34f582edfdd6aeeec76e39ef585c".to_owned(),
            member: String::new(),
        },
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

/// Whether the installed copy is the one this code installed: the archive pin matches, and the
/// binary hashes to the value recorded when it was unpacked (a binary edited or replaced since
/// is fetched again).
fn cached(tool: &Tool, dest: &Path, marker: &Path, bin_marker: &Path) -> bool {
    let pin_matches = matches!(fs::read_to_string(marker), Ok(pin) if pin == tool.sha256);
    let recorded = fs::read_to_string(bin_marker).unwrap_or_default();
    pin_matches
        && matches!(fs::read(dest), Ok(bytes) if !recorded.is_empty() && sha256_hex(&bytes) == recorded)
}

/// Installs `tool` into `bin` unless the pinned version is already there and its binary still
/// has the hash recorded at install.
pub fn ensure(tool: &Tool, bin: &Path, runner: &dyn Runner) -> Result<PathBuf, String> {
    let dest = bin.join(&tool.name);
    let marker = bin.join(format!("{}.sha256", tool.name));
    let bin_marker = bin.join(format!("{}.bin-sha256", tool.name));
    if cached(tool, &dest, &marker, &bin_marker) {
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
    let executable = if tool.url.ends_with(".tar.gz") {
        runner.run(&Cmd::new(
            "tar",
            &["-xzf", &archive_arg, "-C", &unpacked_arg],
            bin,
        ))?;
        unpacked.join(&tool.member)
    } else if tool.url.ends_with(".zst") {
        let plain = unpacked.join(&tool.name);
        runner.run(&Cmd::new(
            "zstd",
            &[
                "--decompress",
                "--quiet",
                "-o",
                &plain.to_string_lossy(),
                &archive_arg,
            ],
            bin,
        ))?;
        plain
    } else {
        archive.clone()
    };
    fs::copy(&executable, &dest).map_err(io_err)?;
    fs::set_permissions(&dest, fs::Permissions::from_mode(0o755)).map_err(io_err)?;
    let installed = fs::read(&dest).map_err(io_err)?;
    fs::write(&bin_marker, sha256_hex(&installed)).map_err(io_err)?;
    fs::write(&marker, &tool.sha256).map_err(io_err)?;
    fs::remove_dir_all(&work).map_err(io_err)?;
    Ok(dest)
}

/// Installs every pinned tool under `root`; each failure is a finding.
pub fn ensure_all(root: &Path, runner: &dyn Runner) -> Vec<Finding> {
    ensure_only(
        root,
        runner,
        &pinned().iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
    )
}

/// Installs the pinned tools named in `names` under `root`; each failure is a finding. A job
/// fetches only what it runs: the static gates need `cargo-deny`, the coverage job
/// `cargo-llvm-cov`, the confirmation job the two build tools.
pub fn ensure_only(root: &Path, runner: &dyn Runner, names: &[String]) -> Vec<Finding> {
    let bin = bin_dir(root);
    let mut out = Vec::new();
    if let Err(e) = fs::create_dir_all(&bin) {
        out.push(Finding::new("fetch", "tools", e.to_string()));
        return out;
    }
    let known = pinned().into_iter().chain(confirmation_tools());
    for tool in known.filter(|t| names.contains(&t.name)) {
        if let Err(e) = ensure(&tool, &bin, runner) {
            out.push(Finding::new("fetch", &tool.name, e));
        }
    }
    out
}
