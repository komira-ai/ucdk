// SPDX-License-Identifier: Apache-2.0
//! The rules about names, enum values, duplicates and the shape of the stage and artifact
//! graphs: what a machine is before anything is asked of what its actions do.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_core::order;
use ucdk_release_proto::v1::action::Kind as ActionKind;
use ucdk_release_proto::v1::{
    ArtifactType, BuildSystemKind, ChannelKind, Machine, Trigger, ValidationKind, Visibility,
};

use super::{
    Findings, Rule, SCHEMA_VERSION, action_platform, builds, identity, needed_names, publishes,
    repeated, smokes, stage_named, validations,
};

pub(super) fn check(m: &Machine, f: &mut Findings) {
    check_versions_and_names(m, f);
    check_enums(m, f);
    check_duplicates(m, f);
    check_actions(m, f);
    check_stages(m, f);
    check_artifact_graph(m, f);
}

fn name_ok(text: &str, underscore: bool) -> bool {
    let bytes = text.as_bytes();
    bytes.len() <= 63
        && bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes.last() != Some(&b'-')
        && bytes.iter().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || (underscore && *b == b'_')
        })
}

fn platform_ok(text: &str) -> bool {
    let part = |p: &str| {
        !p.is_empty()
            && p.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    text == "any"
        || text
            .split_once('-')
            .is_some_and(|(os, cpu)| part(os) && part(cpu))
}

fn check_versions_and_names(m: &Machine, f: &mut Findings) {
    if m.schema_version < SCHEMA_VERSION {
        f.add(
            Rule::SchemaVersion,
            format!(
                "the schema version is {}, and it must be {SCHEMA_VERSION}",
                m.schema_version
            ),
        );
    }
    let mut name = |what: &str, text: &str, underscore: bool| {
        if !name_ok(text, underscore) {
            f.add(Rule::BadName, format!("{what} {text:?} is not a name: lower-case letters, digits and `-` (artifacts also `_`), starting with a letter, at most 63 bytes, not ending in `-`"));
        }
    };
    name("the machine", &m.name, false);
    m.channels
        .iter()
        .for_each(|c| name("the channel", &c.name, false));
    m.artifacts
        .iter()
        .for_each(|a| name("the artifact", &a.name, true));
    for stage in &m.stages {
        name("the stage", &stage.name, false);
        for action in &stage.actions {
            name("the action", &action.name, false);
        }
        for v in validations(stage) {
            name("the validation", &v.name, false);
        }
    }
    for stage in &m.stages {
        for platform in stage.actions.iter().filter_map(action_platform) {
            if !platform_ok(platform) {
                f.add(
                    Rule::BadName,
                    format!(
                        "the platform {platform:?} of stage {} is neither `any` nor `<os>-<cpu>`",
                        stage.name
                    ),
                );
            }
        }
    }
}

fn check_enums(m: &Machine, f: &mut Findings) {
    // An unspecified value is refused by the rule of the thing it types; a number that is no
    // value of the enum at all is `machine.unknown_enum`.
    if let Some(system) = &m.build_system
        && BuildSystemKind::try_from(system.kind).is_err()
    {
        f.add(
            Rule::UnknownEnum,
            format!(
                "the build system has kind {}, which is not a build system kind",
                system.kind
            ),
        );
    }
    for stage in &m.stages {
        if !matches!(
            Trigger::try_from(stage.trigger),
            Ok(Trigger::Release | Trigger::ChangeCheck)
        ) {
            f.add(
                Rule::UnknownEnum,
                format!(
                    "stage {} has trigger {}, which is not a trigger",
                    stage.name, stage.trigger
                ),
            );
        }
        for v in validations(stage) {
            if !matches!(
                ValidationKind::try_from(v.kind),
                Ok(ValidationKind::SetClosure | ValidationKind::ChannelReadback)
            ) {
                f.add(
                    Rule::UnknownEnum,
                    format!(
                        "validation {} has kind {}, which is not a validation kind",
                        v.name, v.kind
                    ),
                );
            }
        }
    }
    for artifact in &m.artifacts {
        match ArtifactType::try_from(artifact.r#type) {
            Ok(ArtifactType::Unspecified) | Err(_) => f.add(
                Rule::UnknownEnum,
                format!(
                    "artifact {} has type {}, which is not an artifact type",
                    artifact.name, artifact.r#type
                ),
            ),
            Ok(kind) if identity(kind).is_none() => f.add(
                Rule::EveryArtifactTypeNamesItsIdentity,
                format!(
                    "artifact {} has type {kind:?}, which does not name its identity yet",
                    artifact.name
                ),
            ),
            Ok(_) => {}
        }
    }
    for channel in &m.channels {
        if ChannelKind::try_from(channel.kind).is_err() {
            f.add(
                Rule::UnknownEnum,
                format!(
                    "channel {} has kind {}, which is not a channel kind",
                    channel.name, channel.kind
                ),
            );
        }
        for repo in &channel.repositories {
            if matches!(
                ArtifactType::try_from(repo.r#type),
                Ok(ArtifactType::Unspecified) | Err(_)
            ) {
                f.add(
                    Rule::UnknownEnum,
                    format!(
                        "repository {} has type {}, which is not an artifact type",
                        repo.location, repo.r#type
                    ),
                );
            }
            if !matches!(
                Visibility::try_from(repo.visibility),
                Ok(Visibility::Public | Visibility::Private)
            ) {
                f.add(
                    Rule::UnknownEnum,
                    format!(
                        "repository {} has visibility {}, which is not a visibility",
                        repo.location, repo.visibility
                    ),
                );
            }
        }
    }
}

fn check_duplicates(m: &Machine, f: &mut Findings) {
    let mut report = |what: &str, names: Vec<&str>| {
        for name in names {
            f.add(
                Rule::DuplicateName,
                format!("the {what} name {name:?} is used twice"),
            );
        }
    };
    report("stage", repeated(m.stages.iter().map(|s| s.name.as_str())));
    report(
        "artifact",
        repeated(m.artifacts.iter().map(|a| a.name.as_str())),
    );
    report(
        "channel",
        repeated(m.channels.iter().map(|c| c.name.as_str())),
    );
    for stage in &m.stages {
        report(
            "action",
            repeated(stage.actions.iter().map(|a| a.name.as_str())),
        );
        report(
            "validation",
            repeated(validations(stage).map(|v| v.name.as_str())),
        );
        for action in &stage.actions {
            let listed = match &action.kind {
                Some(ActionKind::Build(b)) => &b.artifacts,
                Some(ActionKind::Publish(p)) => &p.artifacts,
                Some(ActionKind::Smoke(s)) => &s.artifacts,
                Some(ActionKind::Test(_)) | None => continue,
            };
            report(
                "listed artifact",
                repeated(listed.iter().map(String::as_str)),
            );
        }
    }
}

/// An action has a kind, and a Build, Publish or Smoke action names at least one artifact.
fn check_actions(m: &Machine, f: &mut Findings) {
    for stage in &m.stages {
        if stage.actions.is_empty() {
            f.add(
                Rule::StageWithoutActions,
                format!("stage {} has no action", stage.name),
            );
        }
        for action in &stage.actions {
            let (what, artifacts) = match &action.kind {
                None => {
                    f.add(
                        Rule::ActionWithoutKind,
                        format!(
                            "action {} of stage {} is none of a build, a test, a publish and a smoke",
                            action.name, stage.name
                        ),
                    );
                    continue;
                }
                Some(ActionKind::Build(b)) => ("build", &b.artifacts),
                Some(ActionKind::Publish(p)) => ("publish", &p.artifacts),
                Some(ActionKind::Smoke(s)) => ("smoke", &s.artifacts),
                Some(ActionKind::Test(_)) => continue,
            };
            if artifacts.is_empty() {
                f.add(
                    Rule::EmptyArtifactList,
                    format!(
                        "the {what} action {} of stage {} names no artifact",
                        action.name, stage.name
                    ),
                );
            }
        }
    }
}

fn check_stages(m: &Machine, f: &mut Findings) {
    let position: BTreeMap<&str, usize> = m
        .stages
        .iter()
        .enumerate()
        .map(|(at, s)| (s.name.as_str(), at))
        .collect();
    let needed = needed_names(m);
    for (at, stage) in m.stages.iter().enumerate() {
        for need in &stage.needs {
            match position.get(need.as_str()) {
                None => f.add(
                    Rule::UnknownStage,
                    format!("stage {} needs {need}, which is not declared", stage.name),
                ),
                Some(&other) if other >= at => f.add(
                    Rule::StageCycle,
                    format!(
                        "stage {} needs {need}, which is not declared before it",
                        stage.name
                    ),
                ),
                Some(_) => {}
            }
        }
        let platforms: BTreeSet<&str> = stage.actions.iter().filter_map(action_platform).collect();
        if platforms.len() > 1 {
            f.add(
                Rule::StagePlatform,
                format!(
                    "stage {} names {} platforms, and a stage has one",
                    stage.name,
                    platforms.len()
                ),
            );
        }
        if builds(stage).count() > 1 {
            f.add(
                Rule::StagePlatform,
                format!(
                    "stage {} holds more than one Build action, and a stage seals one set",
                    stage.name
                ),
            );
        }
        if stage.trigger == Trigger::ChangeCheck as i32 {
            let mut reject = |why: &str| {
                f.add(
                    Rule::ChangeCheckPublishes,
                    format!("stage {} is a CHANGE_CHECK stage and {why}", stage.name),
                );
            };
            if publishes(stage).next().is_some() {
                reject("holds a Publish action");
            }
            if smokes(stage).next().is_some() {
                reject("holds a Smoke action");
            }
            if !stage.needs.is_empty() {
                reject("has needs");
            }
            if needed.contains(stage.name.as_str()) {
                reject("is needed by another stage");
            }
            if !stage.environment.is_empty() {
                reject("sets an environment");
            }
            if stage.exceptional {
                reject("is exceptional");
            }
        }
        for need in stage.needs.iter().filter_map(|n| stage_named(m, n)) {
            if need.exceptional != stage.exceptional {
                f.add(
                    Rule::ExceptionalNeedsExceptional,
                    format!(
                        "stage {} and the stage {} it needs are not both exceptional or both not",
                        stage.name, need.name
                    ),
                );
            }
        }
    }
}

fn check_artifact_graph(m: &Machine, f: &mut Findings) {
    let deps: BTreeMap<String, BTreeSet<String>> = m
        .artifacts
        .iter()
        .map(|a| (a.name.clone(), a.needs.iter().cloned().collect()))
        .collect();
    for (name, needs) in &deps {
        for need in needs.iter().filter(|n| !deps.contains_key(*n)) {
            f.add(
                Rule::ArtifactCycle,
                format!("artifact {name} needs {need}, which is not declared"),
            );
        }
    }
    if let Err(members) = order(&deps) {
        f.add(
            Rule::ArtifactCycle,
            format!(
                "artifacts {} need each other in a cycle, or need one that does",
                members.join(", ")
            ),
        );
    }
}
