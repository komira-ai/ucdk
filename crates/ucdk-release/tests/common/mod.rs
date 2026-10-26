// SPDX-License-Identifier: Apache-2.0
//! Fixtures shared by the tests of `ucdk-release`: the project's first machine, which satisfies
//! every rule, and the small builders the tests edit it with.

#![allow(dead_code)]

use std::collections::BTreeMap;

use ucdk_release::machine::{Checked, Rule};
use ucdk_release_proto::v1::action::Kind as ActionKind;
use ucdk_release_proto::v1::push_identity::Kind as PushKind;
use ucdk_release_proto::v1::{
    Action, Anonymous, ArtifactDecl, ArtifactType, Build, BuildStamp, BuildSystem, BuildSystemKind,
    Channel, ChannelKind, Federated, Machine, Publish, PushIdentity, Repository, Smoke, Stage,
    Target, Test, Trigger, Validation, ValidationKind, Visibility,
};

/// The release platform of the example.
pub const PLATFORM: &str = "linux-x86_64";

/// The commit of the examples that hash a revision.
pub const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";

/// The build stamp of the examples.
pub fn stamp() -> BuildStamp {
    BuildStamp {
        version: "1.2.3".to_owned(),
        build_number: 41,
        source_commit: "abc123".to_owned(),
        timestamp_ms: 1_700_000_000_000,
    }
}

pub fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

pub fn pairs(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

pub fn target(label: &str) -> Target {
    Target {
        label: label.to_owned(),
        flags: Vec::new(),
        env: BTreeMap::new(),
    }
}

/// A target with the flags the example's packages are built with.
pub fn stamped_target(label: &str) -> Target {
    Target {
        flags: strings(&[
            "-c",
            "release.version={release_version}",
            "-c",
            "release.commit={source_commit}",
        ]),
        ..target(label)
    }
}

pub fn file_artifact(name: &str, label: &str) -> ArtifactDecl {
    ArtifactDecl {
        name: name.to_owned(),
        r#type: ArtifactType::File as i32,
        needs: Vec::new(),
        target: Some(stamped_target(label)),
        labels: BTreeMap::new(),
    }
}

pub fn validation(name: &str, kind: ValidationKind, wait_polls: u32) -> Validation {
    Validation {
        name: name.to_owned(),
        kind: kind as i32,
        wait_polls,
    }
}

pub fn build_action(
    name: &str,
    platform: &str,
    artifacts: &[&str],
    validations: Vec<Validation>,
) -> Action {
    Action {
        name: name.to_owned(),
        kind: Some(ActionKind::Build(Build {
            platform: platform.to_owned(),
            artifacts: strings(artifacts),
        })),
        validations,
    }
}

pub fn test_action(name: &str, labels: &[&str]) -> Action {
    Action {
        name: name.to_owned(),
        kind: Some(ActionKind::Test(Test {
            targets: labels.iter().map(|l| target(l)).collect(),
        })),
        validations: Vec::new(),
    }
}

pub fn publish_action(name: &str, platform: &str, artifacts: &[&str], channel: &str) -> Action {
    Action {
        name: name.to_owned(),
        kind: Some(ActionKind::Publish(Publish {
            platform: platform.to_owned(),
            artifacts: strings(artifacts),
            channel: channel.to_owned(),
            attempts: 0,
            polls: 0,
        })),
        validations: Vec::new(),
    }
}

pub fn smoke_action(
    name: &str,
    platform: &str,
    artifacts: &[&str],
    channel: &str,
    validations: Vec<Validation>,
) -> Action {
    Action {
        name: name.to_owned(),
        kind: Some(ActionKind::Smoke(Smoke {
            platform: platform.to_owned(),
            artifacts: strings(artifacts),
            channel: channel.to_owned(),
        })),
        validations,
    }
}

pub fn stage(name: &str, trigger: Trigger, needs: &[&str], actions: Vec<Action>) -> Stage {
    Stage {
        name: name.to_owned(),
        needs: strings(needs),
        trigger: trigger as i32,
        environment: String::new(),
        exceptional: false,
        actions,
        validations: Vec::new(),
    }
}

/// A federated identity whose three scopes are all `publish`.
pub fn federated(subject: &str, environment: &str) -> Federated {
    Federated {
        subject: subject.to_owned(),
        audience: "packages.example.com".to_owned(),
        exceptional_subject: String::new(),
        environment: environment.to_owned(),
        exceptional_environment: String::new(),
        pinned_claims: BTreeMap::new(),
        stage_scope: "publish".to_owned(),
        commit_scope: "publish".to_owned(),
        read_scope: String::new(),
    }
}

pub fn federated_push(fed: Federated) -> PushIdentity {
    PushIdentity {
        kind: Some(PushKind::Federated(fed)),
    }
}

pub fn anonymous_push() -> PushIdentity {
    PushIdentity {
        kind: Some(PushKind::Anonymous(Anonymous {})),
    }
}

pub fn repo(
    kind: ArtifactType,
    location: &str,
    visibility: Visibility,
    push: PushIdentity,
) -> Repository {
    Repository {
        r#type: kind as i32,
        location: location.to_owned(),
        visibility: visibility as i32,
        push: Some(push),
    }
}

pub fn channel(name: &str, repositories: Vec<Repository>) -> Channel {
    Channel {
        name: name.to_owned(),
        repositories,
        monotonic: false,
        kind: ChannelKind::PackageRegistry as i32,
    }
}

/// The first machine of the project, as the design writes it: a change check that builds and
/// tests, a release build that seals and tests, and a publish stage that pushes to a package
/// registry and reads the packages back as a stranger would.
pub fn worked_example() -> Machine {
    Machine {
        schema_version: 1,
        name: "ucdk".to_owned(),
        stages: vec![
            stage(
                "check",
                Trigger::ChangeCheck,
                &[],
                vec![
                    build_action("build", PLATFORM, &["ucdk-cli", "ucdk-lib"], Vec::new()),
                    test_action("test", &["//release:set_test"]),
                ],
            ),
            stage(
                "build",
                Trigger::Release,
                &[],
                vec![
                    build_action(
                        "build",
                        PLATFORM,
                        &["ucdk-cli", "ucdk-lib"],
                        vec![validation("closure", ValidationKind::SetClosure, 0)],
                    ),
                    test_action("test", &["//release:set_test"]),
                ],
            ),
            Stage {
                environment: "publish".to_owned(),
                ..stage(
                    "publish",
                    Trigger::Release,
                    &["build"],
                    vec![
                        publish_action("publish", PLATFORM, &["ucdk-cli", "ucdk-lib"], "registry"),
                        smoke_action(
                            "smoke",
                            PLATFORM,
                            &["ucdk-cli", "ucdk-lib"],
                            "registry",
                            vec![validation("visible", ValidationKind::ChannelReadback, 5)],
                        ),
                    ],
                )
            },
        ],
        artifacts: vec![
            file_artifact("ucdk-cli", "//release:ucdk_cli_package"),
            file_artifact("ucdk-lib", "//release:ucdk_lib_package"),
        ],
        channels: vec![channel(
            "registry",
            vec![repo(
                ArtifactType::File,
                "https://packages.example.com/files",
                Visibility::Public,
                federated_push(federated(
                    "repo:example-org/ucdk:environment:publish",
                    "publish",
                )),
            )],
        )],
        stamp_ignore: Vec::new(),
        build_system: Some(BuildSystem {
            kind: BuildSystemKind::Buck2 as i32,
            binary: "buck2".to_owned(),
            config_file: "release/release.buckconfig".to_owned(),
            mode: "release".to_owned(),
            flags: Vec::new(),
            env: pairs(&[("PATH", "/opt/tools/bin:/usr/bin:/bin")]),
        }),
    }
}

/// The machine every rule test edits.
pub fn good() -> Machine {
    worked_example()
}

/// The rules a machine breaks, each once; the machine must be refused.
pub fn broken(machine: Machine) -> Vec<Rule> {
    match Checked::read(machine) {
        Ok(_) => panic!("the machine was read, and it should have been refused"),
        Err(e) => e.rules(),
    }
}

/// `good()` after `edit`, refused: the rules it breaks.
pub fn refused_after(edit: impl FnOnce(&mut Machine)) -> Vec<Rule> {
    let mut machine = good();
    edit(&mut machine);
    broken(machine)
}

/// `good()` after `edit` is refused for `rule` and for no other.
#[track_caller]
pub fn refuses_only(rule: Rule, edit: impl FnOnce(&mut Machine)) {
    assert_eq!(refused_after(edit), [rule], "{}", rule.id());
}

/// `good()` after `edit` is refused for `rule`, and possibly for others the edit also breaks.
#[track_caller]
pub fn refuses(rule: Rule, edit: impl FnOnce(&mut Machine)) {
    let rules = refused_after(edit);
    assert!(rules.contains(&rule), "{}: {rules:?}", rule.id());
}

/// `good()` after `edit` is read.
#[track_caller]
pub fn accepts(edit: impl FnOnce(&mut Machine)) {
    let mut machine = good();
    edit(&mut machine);
    if let Err(e) = Checked::read(machine) {
        panic!("the machine was refused: {e}");
    }
}

pub fn stage_mut<'a>(m: &'a mut Machine, name: &str) -> &'a mut Stage {
    m.stages
        .iter_mut()
        .find(|s| s.name == name)
        .expect("the stage exists")
}

pub fn action_mut<'a>(m: &'a mut Machine, stage: &str, action: &str) -> &'a mut Action {
    stage_mut(m, stage)
        .actions
        .iter_mut()
        .find(|a| a.name == action)
        .expect("the action exists")
}

pub fn artifact_mut<'a>(m: &'a mut Machine, name: &str) -> &'a mut ArtifactDecl {
    m.artifacts
        .iter_mut()
        .find(|a| a.name == name)
        .expect("the artifact exists")
}

pub fn system_mut(m: &mut Machine) -> &mut BuildSystem {
    m.build_system
        .as_mut()
        .expect("the example has a build system")
}

pub fn repo_mut(m: &mut Machine) -> &mut Repository {
    &mut m.channels[0].repositories[0]
}

pub fn federated_mut(m: &mut Machine) -> &mut Federated {
    match &mut repo_mut(m).push.as_mut().expect("push").kind {
        Some(PushKind::Federated(f)) => f,
        _ => panic!("the example pushes as a federated identity"),
    }
}

/// The body of a Publish action of `stage`.
pub fn publish_mut<'a>(m: &'a mut Machine, stage: &str, action: &str) -> &'a mut Publish {
    match &mut action_mut(m, stage, action).kind {
        Some(ActionKind::Publish(p)) => p,
        _ => panic!("{action} is a Publish action"),
    }
}

/// The body of a Smoke action.
pub fn smoke_mut<'a>(m: &'a mut Machine, stage: &str, action: &str) -> &'a mut Smoke {
    match &mut action_mut(m, stage, action).kind {
        Some(ActionKind::Smoke(s)) => s,
        _ => panic!("{action} is a Smoke action"),
    }
}

/// The body of a Build action.
pub fn build_mut<'a>(m: &'a mut Machine, stage: &str, action: &str) -> &'a mut Build {
    match &mut action_mut(m, stage, action).kind {
        Some(ActionKind::Build(b)) => b,
        _ => panic!("{action} is a Build action"),
    }
}

/// The body of a Test action.
pub fn test_mut<'a>(m: &'a mut Machine, stage: &str, action: &str) -> &'a mut Test {
    match &mut action_mut(m, stage, action).kind {
        Some(ActionKind::Test(t)) => t,
        _ => panic!("{action} is a Test action"),
    }
}
