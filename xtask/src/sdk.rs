// SPDX-License-Identifier: Apache-2.0
//! The generated SDK sources: `cargo xtask gen-sdk` writes them, and the `sdk-fresh` gate of
//! `cargo xtask check` holds the committed files to what the generator writes now.
//!
//! The generator (`ucdk-codegen`) returns files; this module decides where they go and compares
//! them. A generated file is committed, so a change to a proto or to the generator shows in
//! review as a diff of the SDK it changes. The gate fails on a file that differs, on one that is
//! missing, and on a `.py` file in the generated directory that the generator no longer writes.

use std::fs;
use std::path::{Path, PathBuf};

use ucdk_codegen::File;

use crate::cmd::{Cmd, Runner};
use crate::{Finding, io_err};

/// Where the Python SDK's package directory is, relative to the workspace root.
pub const PYTHON_DIR: &str = "sdk/python";

fn generated_dir(root: &Path) -> PathBuf {
    root.join(PYTHON_DIR)
        .join(ucdk_codegen::python::GENERATED_DIR)
}

/// The files the generator writes now.
pub fn generate(root: &Path) -> Result<Vec<File>, String> {
    ucdk_codegen::python_for(root).map_err(|e| e.to_string())
}

/// The `.py` files in the generated directory, by name.
fn present(root: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(generated_dir(root)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| Path::new(n).extension().is_some_and(|e| e == "py"))
        .collect();
    names.sort();
    names
}

fn name_of(file: &File) -> &str {
    file.path.rsplit('/').next().unwrap_or(&file.path)
}

/// Writes the generated files, and removes the `.py` files there that are not generated any
/// more. Returns the paths written.
pub fn write(root: &Path) -> Result<Vec<String>, String> {
    let files = generate(root)?;
    fs::create_dir_all(generated_dir(root)).map_err(io_err)?;
    for stale in present(root) {
        if !files.iter().any(|f| name_of(f) == stale) {
            fs::remove_file(generated_dir(root).join(stale)).map_err(io_err)?;
        }
    }
    let mut written = Vec::new();
    for file in &files {
        let path = root.join(PYTHON_DIR).join(&file.path);
        fs::write(&path, &file.text).map_err(io_err)?;
        written.push(format!("{PYTHON_DIR}/{}", file.path));
    }
    Ok(written)
}

/// A finding for every committed generated file that is not what the generator writes now.
pub fn check_fresh(root: &Path) -> (Vec<Finding>, usize) {
    let files = match generate(root) {
        Ok(files) => files,
        Err(e) => return (vec![Finding::new("sdk-generate", PYTHON_DIR, e)], 0),
    };
    let mut findings = Vec::new();
    for file in &files {
        let subject = format!("{PYTHON_DIR}/{}", file.path);
        match fs::read_to_string(root.join(PYTHON_DIR).join(&file.path)) {
            Err(_) => findings.push(Finding::new(
                "sdk-missing",
                subject,
                "not committed; run `cargo xtask gen-sdk`",
            )),
            Ok(text) if text != file.text => findings.push(Finding::new(
                "sdk-stale",
                subject,
                "differs from what the generator writes; run `cargo xtask gen-sdk`",
            )),
            Ok(_) => {}
        }
    }
    for extra in present(root) {
        if !files.iter().any(|f| name_of(f) == extra) {
            findings.push(Finding::new(
                "sdk-extra",
                format!(
                    "{PYTHON_DIR}/{}/{extra}",
                    ucdk_codegen::python::GENERATED_DIR
                ),
                "not written by the generator; run `cargo xtask gen-sdk`",
            ));
        }
    }
    (findings, files.len())
}

/// `cargo xtask sdk-test`: the Python tests of the Python SDK, with the standard library's
/// `unittest` (no package is installed, nothing is fetched). Returns the findings and what was
/// examined. Python tests are allowed under `sdk/python` and nowhere else.
pub fn test(root: &Path, runner: &dyn Runner) -> (Vec<Finding>, String) {
    let dir = root.join(PYTHON_DIR);
    let files = test_files(&dir);
    let examined = format!(
        "{} test file{} under {PYTHON_DIR}/tests",
        files.len(),
        if files.len() == 1 { "" } else { "s" }
    );
    // `-B`: no bytecode files in the tree.
    let run = Cmd::new(
        "python3",
        &["-B", "-m", "unittest", "discover", "-s", "tests", "-t", "."],
        &dir,
    );
    match runner.run(&run) {
        Ok(_) => (Vec::new(), examined),
        Err(e) => (vec![Finding::new("sdk-test", run.to_string(), e)], examined),
    }
}

/// The `test_*.py` files of the SDK's tests directory.
fn test_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir.join("tests")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("test_") && n.ends_with(".py"))
        .collect();
    names.sort();
    names
}
