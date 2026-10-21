// SPDX-License-Identifier: Apache-2.0
//! Every pure crate carries a `clippy.toml` that equals the template.

use std::fs;
use std::path::Path;

use crate::deps::Workspace;
use crate::{Finding, to_result};

pub const TEMPLATE_PATH: &str = "xtask/templates/pure_clippy.toml";

pub fn check_purity(ws: &Workspace, template: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for p in ws.packages.iter().filter(|p| p.pure) {
        let path = p.manifest_dir.join("clippy.toml");
        match fs::read_to_string(&path) {
            Err(_) => out.push(Finding::new(
                "purity-missing",
                &p.name,
                format!("pure crate has no clippy.toml; copy {TEMPLATE_PATH}"),
            )),
            Ok(got) if got != template => out.push(Finding::new(
                "purity-drift",
                &p.name,
                format!("clippy.toml differs from {TEMPLATE_PATH}"),
            )),
            Ok(_) => {}
        }
    }
    out
}

pub fn run(root: &Path) -> Result<(), Vec<Finding>> {
    let ws =
        Workspace::load(root).map_err(|e| vec![Finding::new("metadata", "cargo metadata", e)])?;
    let template = fs::read_to_string(root.join(TEMPLATE_PATH))
        .map_err(|e| vec![Finding::new("template", TEMPLATE_PATH, e.to_string())])?;
    to_result(check_purity(&ws, &template))
}
