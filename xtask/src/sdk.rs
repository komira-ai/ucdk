// SPDX-License-Identifier: Apache-2.0
//! The generated SDK sources: `cargo xtask gen-sdk` writes them, and the `sdk-fresh` gate of
//! `cargo xtask check` holds the committed files to what the generator writes now.
//!
//! The generator (`ucdk-codegen`) returns files; this module decides where they go and compares
//! them, for each SDK. A generated file is committed, so a change to a proto or to the generator
//! shows in review as a diff of the SDK it changes. The gate fails on a file that differs, on one
//! that is missing, and on a file of the SDK's language in the generated directory that the
//! generator no longer writes.

use std::fs;
use std::path::{Path, PathBuf};

use ucdk_codegen::File;

use crate::cmd::{Cmd, Runner};
use crate::{Finding, fetch, io_err};

/// Where the Python SDK's package directory is, relative to the workspace root.
pub const PYTHON_DIR: &str = "sdk/python";
/// Where the TypeScript SDK's package directory is, relative to the workspace root.
pub const TYPESCRIPT_DIR: &str = "sdk/typescript";

/// An SDK whose sources are generated.
struct Target {
    /// The SDK's directory, relative to the workspace root.
    dir: &'static str,
    /// The directory the generated files are in, relative to the SDK's.
    generated: &'static str,
    /// The extension of a generated file.
    extension: &'static str,
    generate: fn(&Path) -> Result<Vec<File>, ucdk_codegen::Error>,
}

const TARGETS: [Target; 2] = [
    Target {
        dir: PYTHON_DIR,
        generated: ucdk_codegen::python::GENERATED_DIR,
        extension: "py",
        generate: ucdk_codegen::python_for,
    },
    Target {
        dir: TYPESCRIPT_DIR,
        generated: ucdk_codegen::typescript::GENERATED_DIR,
        extension: "ts",
        generate: ucdk_codegen::typescript_for,
    },
];

impl Target {
    fn generated_dir(&self, root: &Path) -> PathBuf {
        root.join(self.dir).join(self.generated)
    }

    /// The files the generator writes now.
    fn files(&self, root: &Path) -> Result<Vec<File>, String> {
        (self.generate)(root).map_err(|e| e.to_string())
    }

    /// The generated-language files in the generated directory, by name.
    fn present(&self, root: &Path) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.generated_dir(root)) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                Path::new(n)
                    .extension()
                    .is_some_and(|e| e == self.extension)
            })
            .collect();
        names.sort();
        names
    }
}

fn name_of(file: &File) -> &str {
    file.path.rsplit('/').next().unwrap_or(&file.path)
}

/// Writes the generated files of every SDK, and removes the files there that are not generated
/// any more. Returns the paths written.
pub fn write(root: &Path) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for target in &TARGETS {
        let files = target.files(root)?;
        fs::create_dir_all(target.generated_dir(root)).map_err(io_err)?;
        for stale in target.present(root) {
            if !files.iter().any(|f| name_of(f) == stale) {
                fs::remove_file(target.generated_dir(root).join(stale)).map_err(io_err)?;
            }
        }
        for file in &files {
            let path = root.join(target.dir).join(&file.path);
            fs::write(&path, &file.text).map_err(io_err)?;
            written.push(format!("{}/{}", target.dir, file.path));
        }
    }
    Ok(written)
}

/// A finding for every committed generated file that is not what the generator writes now.
pub fn check_fresh(root: &Path) -> (Vec<Finding>, usize) {
    let mut findings = Vec::new();
    let mut count = 0;
    for target in &TARGETS {
        let files = match target.files(root) {
            Ok(files) => files,
            Err(e) => return (vec![Finding::new("sdk-generate", target.dir, e)], 0),
        };
        count += files.len();
        for file in &files {
            let subject = format!("{}/{}", target.dir, file.path);
            match fs::read_to_string(root.join(target.dir).join(&file.path)) {
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
        for extra in target.present(root) {
            if !files.iter().any(|f| name_of(f) == extra) {
                findings.push(Finding::new(
                    "sdk-extra",
                    format!("{}/{}/{extra}", target.dir, target.generated),
                    "not written by the generator; run `cargo xtask gen-sdk`",
                ));
            }
        }
    }
    (findings, count)
}

/// `cargo xtask sdk-test`: builds the C library the SDKs load, then runs the Python SDK's tests
/// with the standard library's `unittest` and the TypeScript SDK's with Node's `node:test` (no
/// package is installed, nothing is fetched but the pinned Node). Returns the findings and what
/// was examined. SDK tests are allowed under `sdk/<language>/tests` and nowhere else.
pub fn test(root: &Path, runner: &dyn Runner) -> (Vec<Finding>, String) {
    let python = root.join(PYTHON_DIR);
    let typescript = root.join(TYPESCRIPT_DIR);
    let python_files = test_files(&python, "test_", ".py");
    let typescript_files = test_files(&typescript, "", ".test.ts");
    let examined = format!(
        "{} and {}, against the library built from ucdk-capi",
        test_files_of("Python", python_files.len()),
        test_files_of("TypeScript", typescript_files.len()),
    );
    // The tests load `target/debug/libucdk_capi.<so|dylib>`; the SDKs are given its path.
    let build = Cmd::new(
        "cargo",
        &[
            "build",
            "--locked",
            "-p",
            "ucdk-capi",
            "--features",
            "test-clock",
        ],
        root,
    );
    // `-B`: no bytecode files in the tree.
    let python_run = Cmd::new(
        "python3",
        &["-B", "-m", "unittest", "discover", "-s", "tests", "-t", "."],
        &python,
    );
    if let Err(e) = runner.run(&build) {
        return (
            vec![Finding::new("sdk-test", build.to_string(), e)],
            examined,
        );
    }
    if let Err(e) = runner.run(&python_run) {
        return (
            vec![Finding::new("sdk-test", python_run.to_string(), e)],
            examined,
        );
    }
    let mut findings = fetch::ensure_only(root, runner, &[fetch::NODE.to_owned()]);
    if findings.is_empty() {
        // A test that hangs is a failure after a minute, not a job that runs out its time.
        let mut args = vec!["--test", "--test-timeout=60000"];
        args.extend(typescript_files.iter().map(String::as_str));
        let node = Cmd::new("node", &args, &typescript).with_path(&fetch::bin_dir(root));
        if let Err(e) = runner.run(&node) {
            findings.push(Finding::new("sdk-test", node.to_string(), e));
        }
    }
    (findings, examined)
}

fn test_files_of(language: &str, n: usize) -> String {
    format!("{n} {language} test file{}", if n == 1 { "" } else { "s" })
}

/// The test files of the SDK's tests directory: names that start with `prefix` and end with
/// `suffix`, as paths relative to the SDK's directory.
fn test_files(dir: &Path, prefix: &str, suffix: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir.join("tests")) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix) && n.ends_with(suffix))
        .map(|n| format!("tests/{n}"))
        .collect();
    names.sort();
    names
}
