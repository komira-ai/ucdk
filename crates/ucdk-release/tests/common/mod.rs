// SPDX-License-Identifier: Apache-2.0
//! Fixtures shared by the tests of `ucdk-release`: a machine that satisfies every rule, and the
//! small builders the tests edit it with.

#![allow(dead_code)]

use ucdk_release::machine::{Checked, Rule};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::step::Action;
use ucdk_release_proto::v1::{
    Anonymous, ArtifactDecl, ArtifactType, Build, BuildStamp, Channel, Command, ExpectedRow,
    Federated, Machine, Publish, PushIdentity, Repository, Stage, Step, Subject, Trigger,
    Validation, Visibility,
};

/// An edit of a machine, for the tests that apply a list of them.
pub type Edit = Box<dyn Fn(&mut Machine)>;

pub fn decl(name: &str, kind: ArtifactType, needs: &[&str]) -> ArtifactDecl {
    ArtifactDecl {
        name: name.to_owned(),
        r#type: kind as i32,
        builder: Some(Command {
            executable: "make".to_owned(),
            args: vec![name.to_owned(), "{out_dir}".to_owned()],
        }),
        needs: needs.iter().map(|n| (*n).to_owned()).collect(),
        independent_version: false,
    }
}

pub fn validation(name: &str, validator: &str, subject: Subject) -> Validation {
    Validation {
        name: name.to_owned(),
        validator: validator.to_owned(),
        subject: subject as i32,
        rows: vec![ExpectedRow {
            name: "row".to_owned(),
            asserts: "the set is closed".to_owned(),
        }],
        wait_polls: 0,
        options: Default::default(),
    }
}

pub fn build_step(name: &str, platform: &str, artifacts: &[&str], v: Vec<Validation>) -> Step {
    Step {
        name: name.to_owned(),
        action: Some(Action::Build(Build {
            platform: platform.to_owned(),
            artifacts: artifacts.iter().map(|a| (*a).to_owned()).collect(),
        })),
        validations: v,
    }
}

pub fn publish_step(
    name: &str,
    platform: &str,
    artifacts: &[&str],
    channel: &str,
    v: Vec<Validation>,
) -> Step {
    Step {
        name: name.to_owned(),
        action: Some(Action::Publish(Publish {
            platform: platform.to_owned(),
            artifacts: artifacts.iter().map(|a| (*a).to_owned()).collect(),
            channel: channel.to_owned(),
            attempts: 0,
            polls: 0,
        })),
        validations: v,
    }
}

pub fn stage(name: &str, trigger: Trigger, needs: &[&str], steps: Vec<Step>) -> Stage {
    Stage {
        name: name.to_owned(),
        needs: needs.iter().map(|n| (*n).to_owned()).collect(),
        trigger: trigger as i32,
        environment: String::new(),
        exceptional: false,
        steps,
    }
}

pub fn federated(subject: &str, environment: &str) -> PushIdentity {
    PushIdentity {
        kind: Some(Kind::Federated(Federated {
            subject: subject.to_owned(),
            audience: "registry".to_owned(),
            exceptional_subject: String::new(),
            environment: environment.to_owned(),
            exceptional_environment: String::new(),
        })),
    }
}

pub fn anonymous() -> PushIdentity {
    PushIdentity {
        kind: Some(Kind::Anonymous(Anonymous {})),
    }
}

pub fn repo(kind: ArtifactType, location: &str, push: PushIdentity) -> Repository {
    Repository {
        r#type: kind as i32,
        location: location.to_owned(),
        visibility: Visibility::Private as i32,
        push: Some(push),
    }
}

pub fn channel(name: &str, repositories: Vec<Repository>) -> Channel {
    Channel {
        name: name.to_owned(),
        repositories,
        monotonic: false,
    }
}

/// A machine that satisfies every rule: a change check, a release build of `tool` whose set is
/// closure-checked, and a publish stage that pushes it to one channel and reads it back.
pub fn good() -> Machine {
    Machine {
        schema_version: 1,
        name: "demo".to_owned(),
        stages: vec![
            stage(
                "check",
                Trigger::ChangeCheck,
                &[],
                vec![build_step("compile", "linux-x86_64", &["tool"], vec![])],
            ),
            stage(
                "build",
                Trigger::Release,
                &[],
                vec![build_step(
                    "compile",
                    "linux-x86_64",
                    &["tool"],
                    vec![validation("closure", "set-closure", Subject::LocalSet)],
                )],
            ),
            stage(
                "publish",
                Trigger::Release,
                &["build"],
                vec![publish_step(
                    "push",
                    "linux-x86_64",
                    &["tool"],
                    "stable",
                    vec![validation("readback", "fake", Subject::ChannelReadback)],
                )],
            ),
        ],
        artifacts: vec![decl("tool", ArtifactType::File, &[])],
        channels: vec![channel(
            "stable",
            vec![repo(
                ArtifactType::File,
                "registry.example/tool",
                federated("ci-publish", "publish"),
            )],
        )],
        stamp_ignore: vec!["docs/**".to_owned()],
    }
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

pub fn stage_mut<'a>(m: &'a mut Machine, name: &str) -> &'a mut Stage {
    m.stages
        .iter_mut()
        .find(|s| s.name == name)
        .expect("the stage exists")
}

pub fn stamp() -> BuildStamp {
    BuildStamp {
        version: "1.2.3".to_owned(),
        build_number: 41,
        source_commit: "abc123".to_owned(),
        timestamp_ms: 1_700_000_000_000,
    }
}
