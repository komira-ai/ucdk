// SPDX-License-Identifier: Apache-2.0
//! Dependency direction between workspace crates, read from `cargo metadata --no-deps`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::{Finding, to_result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: Kind,
}

#[derive(Debug, Clone)]
pub struct Package {
    pub name: String,
    pub manifest_dir: PathBuf,
    /// `[package.metadata.ucdk] pure = true`.
    pub pure: bool,
    pub deps: Vec<Dep>,
}

/// The workspace packages and their declared dependencies.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    pub packages: Vec<Package>,
}

#[derive(Deserialize)]
struct RawMeta {
    packages: Vec<RawPkg>,
}

#[derive(Deserialize)]
struct RawPkg {
    name: String,
    manifest_path: PathBuf,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
    #[serde(default)]
    dependencies: Vec<RawDep>,
}

#[derive(Deserialize)]
struct RawDep {
    name: String,
    kind: Option<String>,
}

impl Workspace {
    /// Reads the output of `cargo metadata --format-version 1 --no-deps`.
    pub fn from_metadata_json(json: &str) -> Result<Self, String> {
        let raw: RawMeta =
            serde_json::from_str(json).map_err(|e| format!("cargo metadata: {e}"))?;
        let packages = raw
            .packages
            .into_iter()
            .map(|p| {
                let pure = p
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("ucdk"))
                    .and_then(|u| u.get("pure"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let deps = p
                    .dependencies
                    .into_iter()
                    .map(|d| Dep {
                        name: d.name,
                        kind: match d.kind.as_deref() {
                            Some("dev") => Kind::Dev,
                            Some("build") => Kind::Build,
                            _ => Kind::Normal,
                        },
                    })
                    .collect();
                let manifest_dir = p
                    .manifest_path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default();
                Package {
                    name: p.name,
                    manifest_dir,
                    pure,
                    deps,
                }
            })
            .collect();
        Ok(Self { packages })
    }

    /// Runs `cargo metadata` in `root`.
    pub fn load(root: &Path) -> Result<Self, String> {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
        let out = Command::new(cargo)
            .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
            .current_dir(root)
            .output()
            .map_err(|e| format!("cargo metadata: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "cargo metadata: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        Self::from_metadata_json(&String::from_utf8_lossy(&out.stdout))
    }

    fn has(&self, name: &str) -> bool {
        self.packages.iter().any(|p| p.name == name)
    }

    fn is_pure(&self, name: &str) -> bool {
        self.packages.iter().any(|p| p.name == name && p.pure)
    }
}

/// The dependency rules of the design's crate table. A rule that names a crate that does not
/// exist yet binds nothing; a rule that names an existing crate binds.
#[derive(Debug, Clone, Default)]
pub struct DepRules {
    /// Crates that must declare `pure = true`.
    pub pure: BTreeSet<String>,
    /// Crate -> the only workspace crates it may depend on.
    pub only_deps: BTreeMap<String, BTreeSet<String>>,
    /// Crates that only the `only_via` crate may depend on (the engine and publish crates).
    pub guarded: BTreeSet<String>,
    /// Crate -> the one guarded crate it may reach the guarded set through.
    pub only_via: BTreeMap<String, String>,
    /// Crates that may appear only as a dev-dependency.
    pub dev_only: BTreeSet<String>,
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|n| (*n).to_owned()).collect()
}

impl DepRules {
    /// The rules of the design's crate table.
    pub fn ucdk() -> Self {
        Self {
            pure: set(&[
                "ucdk-proto",
                "ucdk-model",
                "ucdk-plan",
                "ucdk-provider",
                "ucdk-trust",
                "ucdk-release",
                "ucdk-sign",
                "ucdk-api",
                "ucdk-testutil",
            ]),
            only_deps: BTreeMap::from([(
                "ucdk-api".to_owned(),
                set(&["ucdk-proto", "ucdk-model"]),
            )]),
            guarded: set(&[
                "ucdk-engine",
                "ucdk-release",
                "ucdk-channel-apt",
                "ucdk-channel-rpm",
                "ucdk-channel-tap",
                "ucdk-sign",
                "ucdk-validate",
            ]),
            only_via: BTreeMap::from([
                ("ucdk-cli".to_owned(), "ucdk-dispatch".to_owned()),
                ("ucdk-capi".to_owned(), "ucdk-dispatch".to_owned()),
            ]),
            dev_only: set(&["ucdk-testutil"]),
        }
    }
}

/// Every edge (package, dependency) between the workspace's own crates, of every kind.
fn edges(ws: &Workspace) -> impl Iterator<Item = (&Package, &Dep)> {
    ws.packages
        .iter()
        .flat_map(|p| p.deps.iter().map(move |d| (p, d)))
        .filter(|(_, d)| ws.has(&d.name))
}

pub fn check_deps(ws: &Workspace, rules: &DepRules) -> Vec<Finding> {
    let mut out = Vec::new();
    for p in &ws.packages {
        if rules.pure.contains(&p.name) && !p.pure {
            out.push(Finding::new(
                "pure-unmarked",
                &p.name,
                "design lists it as pure but it lacks [package.metadata.ucdk] pure = true",
            ));
        }
    }
    for (p, d) in edges(ws) {
        if p.pure && !ws.is_pure(&d.name) {
            out.push(Finding::new(
                "pure-depends-on-impure",
                &p.name,
                format!("pure crate depends on {} ({:?})", d.name, d.kind),
            ));
        }
        if let Some(only) = rules.only_deps.get(&p.name)
            && !only.contains(&d.name)
        {
            out.push(Finding::new(
                "only-deps",
                &p.name,
                format!("may depend only on {only:?}, but depends on {}", d.name),
            ));
        }
        if let Some(via) = rules.only_via.get(&p.name)
            && rules.guarded.contains(&d.name)
            && &d.name != via
        {
            out.push(Finding::new(
                "only-via",
                &p.name,
                format!("must reach {} through {via}", d.name),
            ));
        }
        if rules.dev_only.contains(&d.name) && d.kind != Kind::Dev {
            out.push(Finding::new(
                "dev-only",
                &p.name,
                format!("{} may only be a dev-dependency", d.name),
            ));
        }
    }
    out
}

/// The `deps` verb: the design's rules over the real workspace.
pub fn run(root: &Path) -> Result<(), Vec<Finding>> {
    let ws =
        Workspace::load(root).map_err(|e| vec![Finding::new("metadata", "cargo metadata", e)])?;
    to_result(check_deps(&ws, &DepRules::ucdk()))
}
