// SPDX-License-Identifier: Apache-2.0
//! The rules about the build system, the targets that name what it builds and tests, and the
//! Build and Test actions that run it. The text rules of flags and environments are the
//! functions of [`crate::flags`], applied here to every layer a machine authors.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_release_proto::v1::{ArtifactType, BuildSystem, BuildSystemKind, Machine, Stage, Target};

use super::{Findings, MAX_TEST_TARGETS, Rule, artifact_named, builds, stage_platform, tests};
use crate::flags::{self, Placeholder};

pub(super) fn check(m: &Machine, f: &mut Findings) {
    let kind = m
        .build_system
        .as_ref()
        .map_or(BuildSystemKind::Unspecified, |s| {
            BuildSystemKind::try_from(s.kind).unwrap_or(BuildSystemKind::Unspecified)
        });
    check_required(m, f);
    if let Some(system) = &m.build_system {
        check_system_shape(system, f);
        check_layer("the build system", &system.flags, &system.env, kind, f);
    }
    check_artifacts(m, kind, f);
    check_builds(m, f);
    check_tests(m, kind, f);
}

fn check_required(m: &Machine, f: &mut Findings) {
    if m.build_system.is_some() {
        return;
    }
    for stage in &m.stages {
        if builds(stage).next().is_some() || tests(stage).next().is_some() {
            f.add(
                Rule::BuildSystemRequired,
                format!(
                    "stage {} holds a Build or a Test action, and the machine has no build system",
                    stage.name
                ),
            );
        }
    }
}

fn check_system_shape(system: &BuildSystem, f: &mut Findings) {
    let mut refuse = |why: String| f.add(Rule::BuildSystemShape, format!("the build system {why}"));
    if BuildSystemKind::try_from(system.kind) == Ok(BuildSystemKind::Unspecified) {
        refuse("has no kind".to_owned());
    }
    let binary = system.binary.as_str();
    if binary.is_empty() {
        refuse("has no binary".to_owned());
    } else if !binary.starts_with('/') {
        if binary.contains('/') {
            refuse(format!(
                "has the binary {binary:?}, which is neither a bare name nor an absolute path"
            ));
        } else if !system.env.contains_key("PATH") {
            refuse(format!(
                "has the bare binary {binary:?} and no PATH in its environment to find it on"
            ));
        }
    }
    if system.config_file.starts_with('/') || system.config_file.split('/').any(|p| p == "..") {
        refuse(format!(
            "has the config file {:?}, which must be relative to the root and stay under it",
            system.config_file
        ));
    }
    if let Some(bad) = system
        .mode
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || "._/@=-".contains(*c)))
    {
        refuse(format!(
            "has the mode {:?}, which holds {bad:?}; a mode is [A-Za-z0-9._/@=-]",
            system.mode
        ));
    }
    for (at, flag) in system.flags.iter().enumerate() {
        if flag.is_empty() || flag.contains('\0') {
            refuse(format!("has flag {at}, which is empty or holds a NUL"));
        }
    }
}

/// The text rules over one layer of authored flags and environment: the build system's own, or
/// one target's.
fn check_layer(
    whose: &str,
    flag_list: &[String],
    env: &BTreeMap<String, String>,
    kind: BuildSystemKind,
    f: &mut Findings,
) {
    for flag in flag_list {
        if let Some(why) = flags::reserved(kind, flag) {
            f.add(
                Rule::ReservedFlag,
                format!("{whose} has the flag {flag:?}: {why}"),
            );
        }
        if let Err(e) = flags::placeholders(flag) {
            f.add(
                Rule::Placeholder,
                format!("{whose} has the flag {flag:?}: {e}"),
            );
        }
    }
    for (key, value) in env {
        if let Err(e) = flags::env_key(key) {
            f.add(
                Rule::EnvKey,
                format!("{whose} has the environment key {key:?}: {e}"),
            );
        }
        if let Err(e) = flags::placeholders(value) {
            f.add(
                Rule::Placeholder,
                format!("{whose} has the environment value of {key}: {e}"),
            );
        }
    }
}

/// Why `label` is not exactly one label of the grammar of `kind`: Buck2 `[cell]//package:name`,
/// Bazel `[@repo]//package:name`.
fn label_problem(kind: BuildSystemKind, label: &str) -> Option<String> {
    if label.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Some("holds a space or a control character".to_owned());
    }
    if label.contains('*') || label.contains("...") {
        return Some("holds a wildcard, and a target is exactly one label".to_owned());
    }
    let Some((root, rest)) = label.split_once("//") else {
        return Some("has no `//`".to_owned());
    };
    let root_ok = match kind {
        BuildSystemKind::Buck2 | BuildSystemKind::Unspecified => root
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c)),
        BuildSystemKind::Bazel => {
            // `@//pkg:name` names the main repository, so an empty repository is allowed.
            root.is_empty()
                || root
                    .strip_prefix("@@")
                    .or_else(|| root.strip_prefix('@'))
                    .is_some_and(|repo| {
                        repo.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "_.~+-".contains(c))
                    })
        }
    };
    if !root_ok {
        return Some(format!(
            "has the cell or repository {root:?}, which is not one"
        ));
    }
    let Some((package, name)) = rest.split_once(':') else {
        return Some("has no `:name`".to_owned());
    };
    if name.is_empty() || name.contains(':') {
        return Some("has no single target name after the `:`".to_owned());
    }
    if name == "all" {
        return Some("is the wildcard `:all`".to_owned());
    }
    if !package.is_empty()
        && package
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Some(format!("has the package {package:?}, which is not a path"));
    }
    None
}

fn check_target(whose: &str, target: &Target, kind: BuildSystemKind, f: &mut Findings) {
    if let Some(why) = label_problem(kind, &target.label) {
        f.add(
            Rule::TargetShape,
            format!("{whose} has the label {:?}, which {why}", target.label),
        );
    }
    check_layer(whose, &target.flags, &target.env, kind, f);
}

fn check_artifacts(m: &Machine, kind: BuildSystemKind, f: &mut Findings) {
    for artifact in &m.artifacts {
        match &artifact.target {
            None => f.add(
                Rule::ArtifactTargetMissing,
                format!("artifact {} has no target", artifact.name),
            ),
            Some(target) => check_target(
                &format!("the target of artifact {}", artifact.name),
                target,
                kind,
                f,
            ),
        }
        if !artifact.labels.is_empty() && artifact.r#type != ArtifactType::File as i32 {
            f.add(
                Rule::LabelsOnNonFile,
                format!("artifact {} has labels and is not a FILE", artifact.name),
            );
        }
    }
}

fn check_builds(m: &Machine, f: &mut Findings) {
    for stage in &m.stages {
        for (action, build) in builds(stage) {
            for name in build
                .artifacts
                .iter()
                .filter(|n| artifact_named(m, n).is_none())
            {
                f.add(
                    Rule::BuildUndeclared,
                    format!(
                        "action {} builds {name}, which is not declared",
                        action.name
                    ),
                );
            }
            let listed: BTreeSet<&str> = build.artifacts.iter().map(String::as_str).collect();
            for artifact in build.artifacts.iter().filter_map(|n| artifact_named(m, n)) {
                for need in artifact
                    .needs
                    .iter()
                    .filter(|n| artifact_named(m, n).is_some() && !listed.contains(n.as_str()))
                {
                    f.add(
                        Rule::BuildNotClosed,
                        format!(
                            "action {} builds {}, which needs {need}, and does not build it",
                            action.name, artifact.name
                        ),
                    );
                }
            }
        }
    }
}

fn check_tests(m: &Machine, kind: BuildSystemKind, f: &mut Findings) {
    for stage in &m.stages {
        for (action, test) in tests(stage) {
            let whose = format!("action {} of stage {}", action.name, stage.name);
            if test.targets.is_empty() || test.targets.len() > MAX_TEST_TARGETS {
                f.add(
                    Rule::TestShape,
                    format!(
                        "{whose} has {} targets, and a test has 1 to {MAX_TEST_TARGETS}",
                        test.targets.len()
                    ),
                );
            }
            let mut seen = BTreeSet::new();
            for target in &test.targets {
                if !seen.insert(target.label.as_str()) {
                    f.add(
                        Rule::TestShape,
                        format!("{whose} names the target {:?} twice", target.label),
                    );
                }
                check_target(&format!("a target of {whose}"), target, kind, f);
            }
            if !action.validations.is_empty() {
                f.add(
                    Rule::TestShape,
                    format!("{whose} carries validations, and its targets are its validations"),
                );
            }
            check_test_platform(m, stage, &whose, test.targets.as_slice(), f);
        }
    }
}

/// `{platform}` in a test of a stage that names no platform has nothing to stand for. The
/// build system's own flags and environment are layered into every test invocation, so they
/// count as well as the target's.
fn check_test_platform(
    m: &Machine,
    stage: &Stage,
    whose: &str,
    targets: &[Target],
    f: &mut Findings,
) {
    if stage_platform(stage).is_some() {
        return;
    }
    let uses_platform = |flag_list: &[String], env: &BTreeMap<String, String>| {
        flag_list
            .iter()
            .chain(env.values())
            .filter_map(|text| flags::placeholders(text).ok())
            .any(|found| found.contains(&Placeholder::Platform))
    };
    let base = m
        .build_system
        .as_ref()
        .is_some_and(|s| uses_platform(&s.flags, &s.env));
    if base || targets.iter().any(|t| uses_platform(&t.flags, &t.env)) {
        f.add(
            Rule::Placeholder,
            format!("{whose} uses {{platform}}, and its stage names no platform"),
        );
    }
}
