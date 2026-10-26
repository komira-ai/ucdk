// SPDX-License-Identifier: Apache-2.0
//! Reading a machine: one test per rule id, each refusing a machine that breaks that rule and
//! nothing else, and the positive cases that keep a rule from refusing too much.

mod common;

use common::*;
use ucdk_release::machine::{
    Checked, DEFAULT_PUT_ATTEMPTS, DEFAULT_READBACK_POLLS, Identity, ReadError, Rule, identity,
    needed_builds, put_attempts, readback_polls, stage_environment, stage_platform, step_platform,
};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::step::Action;
use ucdk_release_proto::v1::{ArtifactType, Build, Machine, Reason, Step, Subject, Trigger};

fn stable_repo(m: &mut Machine) -> &mut ucdk_release_proto::v1::Repository {
    &mut m.channels[0].repositories[0]
}

fn federated_of(m: &mut Machine) -> &mut ucdk_release_proto::v1::Federated {
    match &mut stable_repo(m).push.as_mut().expect("push").kind {
        Some(Kind::Federated(f)) => f,
        _ => panic!("the fixture pushes as a federated identity"),
    }
}

/// A second build stage and a second publish stage of the same machine, to the channel `beta`.
fn with_second_publish(m: &mut Machine, needs: &[&str]) {
    let mut build2 = stage_mut(m, "build").clone();
    build2.name = "build2".to_owned();
    m.stages.push(build2);
    m.channels.push(channel(
        "beta",
        vec![repo(
            ArtifactType::File,
            "registry.example/beta",
            federated("ci-beta", "publish2"),
        )],
    ));
    m.stages.push(stage(
        "publish2",
        Trigger::Release,
        needs,
        vec![publish_step(
            "push",
            "linux-x86_64",
            &["tool"],
            "beta",
            vec![],
        )],
    ));
}

#[test]
fn the_good_machine_is_read_and_its_views_answer() {
    let checked = Checked::read(good()).expect("the machine satisfies every rule");
    assert_eq!(checked.machine().name, "demo");
    assert_eq!(checked.stage("build").expect("stage").name, "build");
    assert!(checked.stage("ghost").is_none());
    assert_eq!(checked.artifact("tool").expect("artifact").name, "tool");
    assert!(checked.artifact("ghost").is_none());
    assert_eq!(checked.channel("stable").expect("channel").name, "stable");
    assert!(checked.channel("ghost").is_none());
}

#[test]
fn newer_major_is_a_version_error() {
    let mut m = good();
    m.schema_version = 2;
    let err = Checked::read(m).expect_err("a newer schema is refused");
    assert_eq!(err, ReadError::Version { found: 2 });
    assert_eq!(err.reason(), Reason::MachineVersion);
    assert!(err.rules().is_empty());
    assert!(err.to_string().contains("schema version 2"));
}

#[test]
fn a_missing_schema_version_is_refused() {
    let mut m = good();
    m.schema_version = 0;
    let err = Checked::read(m).expect_err("refused");
    assert_eq!(err.rules(), [Rule::SchemaVersion]);
    assert_eq!(err.reason(), Reason::MachineFormat);
}

#[test]
fn a_refusal_names_its_rule_in_its_text() {
    let mut m = good();
    m.schema_version = 0;
    m.name = "Demo".to_owned();
    let err = Checked::read(m).expect_err("refused");
    let text = err.to_string();
    assert!(text.contains("machine.schema_version: "), "{text}");
    assert!(text.contains("machine.bad_name: "), "{text}");
    assert_eq!(err.rules(), [Rule::SchemaVersion, Rule::BadName]);
}

#[test]
fn every_rule_has_a_distinct_stable_id() {
    let mut ids: Vec<&str> = Rule::ALL.iter().map(|r| r.id()).collect();
    assert!(ids.iter().all(|id| id.starts_with("machine.")));
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), Rule::ALL.len());
    let mut sorted = Rule::ALL;
    sorted.sort();
    assert_eq!(sorted, Rule::ALL, "the list is in the order of the enum");
}

#[test]
fn a_name_follows_the_grammar() {
    let long = "a".repeat(64);
    for bad in ["", "9tool", "Tool", "a_b", "a b", "tool-", &long] {
        assert_eq!(
            refused_after(|m| m.name = bad.to_owned()),
            [Rule::BadName],
            "{bad:?}"
        );
    }
    let longest = "a".repeat(63);
    for ok in ["a", "a-b", "a1", longest.as_str()] {
        let mut m = good();
        m.name = ok.to_owned();
        Checked::read(m).unwrap_or_else(|e| panic!("{ok:?}: {e}"));
    }
}

#[test]
fn every_kind_of_name_is_checked() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "check").name = "Check".to_owned()),
        [Rule::BadName]
    );
    assert_eq!(
        refused_after(|m| stage_mut(m, "check").steps[0].name = "Compile".to_owned()),
        [Rule::BadName]
    );
    assert_eq!(
        refused_after(|m| stage_mut(m, "build").steps[0].validations[0].name = "Closure".to_owned()),
        [Rule::BadName]
    );
    assert_eq!(
        refused_after(|m| m.channels.push(channel(
            "Extra",
            vec![repo(
                ArtifactType::File,
                "registry.example/extra",
                anonymous()
            )]
        ))),
        [Rule::BadName]
    );
    assert_eq!(
        refused_after(|m| m.artifacts.push(decl("Lib", ArtifactType::File, &[]))),
        [Rule::BadName]
    );
}

#[test]
fn an_artifact_name_may_hold_an_underscore_and_a_stage_name_may_not() {
    let mut m = good();
    m.artifacts.push(decl("lib_two", ArtifactType::File, &[]));
    Checked::read(m).expect("an artifact name allows `_`");
    assert_eq!(
        refused_after(|m| stage_mut(m, "check").name = "check_it".to_owned()),
        [Rule::BadName]
    );
}

#[test]
fn a_platform_is_any_or_os_dash_cpu() {
    for bad in [
        "linux",
        "",
        "Linux-x86_64",
        "linux-",
        "-x86",
        "linux-x86-64",
    ] {
        assert_eq!(
            refused_after(|m| {
                let Some(Action::Build(b)) = &mut stage_mut(m, "check").steps[0].action else {
                    panic!("a build step");
                };
                b.platform = bad.to_owned();
            }),
            [Rule::BadName],
            "{bad:?}"
        );
    }
    for ok in ["any", "osx-arm64", "linux-x86_64"] {
        let mut m = good();
        let Some(Action::Build(b)) = &mut stage_mut(&mut m, "check").steps[0].action else {
            panic!("a build step");
        };
        b.platform = ok.to_owned();
        Checked::read(m).unwrap_or_else(|e| panic!("{ok:?}: {e}"));
    }
}

#[test]
fn an_unknown_enum_value_is_refused() {
    for trigger in [0, 7, -1] {
        assert_eq!(
            refused_after(|m| stage_mut(m, "check").trigger = trigger),
            [Rule::UnknownEnum],
            "trigger {trigger}"
        );
    }
    for subject in [0, 9] {
        assert_eq!(
            refused_after(|m| {
                let mut v = validation("odd", "fake", Subject::LocalSet);
                v.subject = subject;
                stage_mut(m, "check").steps[0].validations.push(v);
            }),
            [Rule::UnknownEnum],
            "subject {subject}"
        );
    }
    for kind in [0, 99] {
        assert_eq!(
            refused_after(|m| {
                let mut d = decl("odd", ArtifactType::File, &[]);
                d.r#type = kind;
                m.artifacts.push(d);
            }),
            [Rule::UnknownEnum],
            "artifact type {kind}"
        );
        assert_eq!(
            refused_after(|m| stable_repo(m).r#type = kind),
            [Rule::UnknownEnum, Rule::PublishUndeclared],
            "repository type {kind}"
        );
    }
    for visibility in [0, 5] {
        assert_eq!(
            refused_after(|m| stable_repo(m).visibility = visibility),
            [Rule::UnknownEnum],
            "visibility {visibility}"
        );
    }
}

#[test]
fn a_step_with_no_action_is_refused() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "check").steps.push(Step {
            name: "idle".to_owned(),
            action: None,
            validations: vec![],
        })),
        [Rule::StepWithoutAction]
    );
}

#[test]
fn duplicate_stage_name_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let again = stage_mut(m, "check").clone();
            m.stages.push(again);
        }),
        [Rule::DuplicateName]
    );
}

#[test]
fn duplicate_step_name_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let again = stage_mut(m, "check").steps[0].clone();
            stage_mut(m, "check").steps.push(again);
        }),
        [Rule::DuplicateName, Rule::StagePlatform]
    );
}

#[test]
fn duplicate_validation_name_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let again = validation("closure", "other", Subject::LocalSet);
            stage_mut(m, "build").steps[0].validations.push(again);
        }),
        [Rule::DuplicateName]
    );
}

#[test]
fn duplicate_artifact_name_is_refused() {
    assert_eq!(
        refused_after(|m| m.artifacts.push(decl("tool", ArtifactType::File, &[]))),
        [Rule::DuplicateName]
    );
}

#[test]
fn duplicate_channel_name_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let mut again = m.channels[0].clone();
            again.repositories[0].location = "registry.example/other".to_owned();
            m.channels.push(again);
        }),
        [Rule::DuplicateName]
    );
}

#[test]
fn a_stage_needing_itself_or_a_later_stage_is_a_cycle() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "build").needs = vec!["build".to_owned()]),
        [Rule::StageCycle]
    );
    assert_eq!(
        refused_after(|m| stage_mut(m, "build").needs = vec!["publish".to_owned()]),
        [Rule::StageCycle]
    );
}

#[test]
fn a_stage_needing_an_undeclared_stage_is_refused() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "publish").needs.push("ghost".to_owned())),
        [Rule::UnknownStage]
    );
}

#[test]
fn a_publish_to_an_undeclared_channel_is_refused_and_lists_the_declared_ones() {
    let mut m = good();
    let Some(Action::Publish(p)) = &mut stage_mut(&mut m, "publish").steps[0].action else {
        panic!("a publish step");
    };
    p.channel = "ghost".to_owned();
    let err = Checked::read(m).expect_err("refused");
    assert_eq!(err.rules(), [Rule::UnknownChannel]);
    assert!(err.to_string().contains("[stable]"), "{err}");
}

#[test]
fn change_check_that_publishes_is_refused() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "check").steps = vec![publish_step(
            "push",
            "linux-x86_64",
            &["tool"],
            "stable",
            vec![]
        )]),
        [
            Rule::ChangeCheckPublishes,
            Rule::EnvironmentMismatch,
            Rule::PublishStageNeedsValidatedBuild
        ]
    );
}

#[test]
fn change_check_with_needs_is_refused() {
    assert_eq!(
        refused_after(|m| {
            m.stages
                .insert(0, stage("pre", Trigger::Release, &[], vec![]));
            stage_mut(m, "check").needs = vec!["pre".to_owned()];
        }),
        [Rule::ChangeCheckPublishes]
    );
}

#[test]
fn change_check_that_another_stage_needs_is_refused() {
    assert_eq!(
        refused_after(|m| m
            .stages
            .push(stage("gate", Trigger::Release, &["check"], vec![]))),
        [Rule::ChangeCheckPublishes]
    );
}

#[test]
fn change_check_with_environment_is_refused() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "check").environment = "review".to_owned()),
        [Rule::ChangeCheckPublishes]
    );
}

#[test]
fn change_check_that_is_exceptional_is_refused() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "check").exceptional = true),
        [Rule::ChangeCheckPublishes]
    );
}

#[test]
fn a_stage_with_two_platforms_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let mut second = stage_mut(m, "publish").steps[0].clone();
            second.name = "push-arm".to_owned();
            second.validations.clear();
            let Some(Action::Publish(p)) = &mut second.action else {
                panic!("a publish step");
            };
            p.platform = "linux-aarch64".to_owned();
            stage_mut(m, "publish").steps.push(second);
        }),
        [Rule::StagePlatform, Rule::PublishStageScope]
    );
}

#[test]
fn a_stage_with_two_builds_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let mut second = stage_mut(m, "check").steps[0].clone();
            second.name = "compile-again".to_owned();
            stage_mut(m, "check").steps.push(second);
        }),
        [Rule::StagePlatform]
    );
}

#[test]
fn an_artifact_needs_cycle_is_refused() {
    assert_eq!(
        refused_after(|m| {
            m.artifacts[0].needs = vec!["lib".to_owned()];
            m.artifacts.push(decl("lib", ArtifactType::File, &["tool"]));
        }),
        [Rule::ArtifactCycle]
    );
}

#[test]
fn an_artifact_need_that_is_not_declared_is_refused() {
    assert_eq!(
        refused_after(|m| m.artifacts[0].needs = vec!["ghost".to_owned()]),
        [Rule::ArtifactCycle]
    );
}

#[test]
fn a_channel_with_no_repository_is_refused() {
    assert_eq!(
        refused_after(|m| m.channels.push(channel("empty", vec![]))),
        [Rule::ChannelShape]
    );
}

#[test]
fn two_repositories_of_one_type_in_a_channel_are_refused() {
    assert_eq!(
        refused_after(|m| m.channels[0].repositories.push(repo(
            ArtifactType::File,
            "registry.example/second",
            federated("ci-publish", "publish")
        ))),
        [Rule::ChannelShape]
    );
}

#[test]
fn a_location_shared_by_two_repositories_is_refused() {
    assert_eq!(
        refused_after(|m| m.channels[0].repositories.push(repo(
            ArtifactType::Oci,
            "registry.example/tool",
            anonymous()
        ))),
        [Rule::ChannelShape]
    );
    assert_eq!(
        refused_after(|m| m.channels.push(channel(
            "other",
            vec![repo(
                ArtifactType::File,
                "registry.example/tool",
                anonymous()
            )]
        ))),
        [Rule::ChannelShape]
    );
}

#[test]
fn a_repository_with_no_push_identity_is_refused() {
    assert_eq!(
        refused_after(|m| stable_repo(m).push = None),
        [Rule::ChannelShape]
    );
    assert_eq!(
        refused_after(|m| stable_repo(m).push = Some(Default::default())),
        [Rule::ChannelShape]
    );
}

#[test]
fn an_exceptional_subject_equal_to_the_subject_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let f = federated_of(m);
            f.exceptional_subject = f.subject.clone();
            f.exceptional_environment = "exceptional".to_owned();
        }),
        [Rule::ExceptionalSubject]
    );
}

#[test]
fn an_exceptional_environment_equal_to_the_environment_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let f = federated_of(m);
            f.exceptional_subject = "ci-exceptional".to_owned();
            f.exceptional_environment = f.environment.clone();
        }),
        [Rule::ExceptionalSubject]
    );
}

#[test]
fn exceptional_environment_without_its_subject_is_refused() {
    assert_eq!(
        refused_after(|m| federated_of(m).exceptional_environment = "exceptional".to_owned()),
        [Rule::ExceptionalSubject]
    );
}

#[test]
fn exceptional_subject_without_its_environment_is_refused() {
    assert_eq!(
        refused_after(|m| federated_of(m).exceptional_subject = "ci-exceptional".to_owned()),
        [Rule::ExceptionalSubject]
    );
}

#[test]
fn a_build_of_an_undeclared_artifact_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let Some(Action::Build(b)) = &mut stage_mut(m, "check").steps[0].action else {
                panic!("a build step");
            };
            b.artifacts.push("ghost".to_owned());
        }),
        [Rule::BuildUndeclared]
    );
}

#[test]
fn a_publish_of_an_undeclared_artifact_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let Some(Action::Publish(p)) = &mut stage_mut(m, "publish").steps[0].action else {
                panic!("a publish step");
            };
            p.artifacts.push("ghost".to_owned());
        }),
        [Rule::PublishUndeclared, Rule::PublishStageScope]
    );
}

#[test]
fn a_publish_to_a_channel_with_no_repository_for_the_type_is_refused() {
    assert_eq!(
        refused_after(|m| {
            m.artifacts.push(decl("image", ArtifactType::Oci, &[]));
            for name in ["build", "publish"] {
                match &mut stage_mut(m, name).steps[0].action {
                    Some(Action::Build(Build { artifacts, .. })) => {
                        artifacts.push("image".to_owned());
                    }
                    Some(Action::Publish(p)) => p.artifacts.push("image".to_owned()),
                    None => panic!("an action"),
                }
            }
        }),
        [Rule::PublishUndeclared]
    );
}

#[test]
fn a_channel_readback_on_a_build_step_is_refused() {
    assert_eq!(
        refused_after(
            |m| stage_mut(m, "check").steps[0].validations.push(validation(
                "look",
                "fake",
                Subject::ChannelReadback
            ))
        ),
        [Rule::ReadbackOnBuild]
    );
}

#[test]
fn a_local_set_validation_with_no_build_before_it_is_refused() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "publish").steps[0]
            .validations
            .push(validation("closure", "set-closure", Subject::LocalSet))),
        [Rule::LocalSetWithoutBuild]
    );
}

#[test]
fn a_local_set_validation_counts_a_build_earlier_in_the_stage() {
    // The stage holds a build and a publish, which is refused for that; the local-set
    // validation on the later step is not refused, because a build comes before it.
    assert_eq!(
        refused_after(|m| {
            let publish = stage_mut(m, "publish");
            publish.steps[0].validations.push(validation(
                "closure",
                "set-closure",
                Subject::LocalSet,
            ));
            publish
                .steps
                .insert(0, build_step("extra", "linux-x86_64", &["tool"], vec![]));
        }),
        [Rule::PublishStageHasBuild]
    );
}

#[test]
fn environment_mismatch_is_refused() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "publish").environment = "other".to_owned()),
        [Rule::EnvironmentMismatch]
    );
    assert_eq!(
        refused_after(|m| federated_of(m).environment = String::new()),
        [Rule::EnvironmentMismatch]
    );
}

#[test]
fn an_exceptional_stage_may_run_in_the_exceptional_environment_and_in_no_other() {
    let exceptional = |m: &mut Machine, env: &str| {
        let f = federated_of(m);
        f.exceptional_subject = "ci-exceptional".to_owned();
        f.exceptional_environment = "publish-exceptional".to_owned();
        for name in ["build", "publish"] {
            stage_mut(m, name).exceptional = true;
        }
        stage_mut(m, "publish").environment = env.to_owned();
    };
    for ok in ["publish-exceptional", "publish"] {
        let mut m = good();
        exceptional(&mut m, ok);
        Checked::read(m).unwrap_or_else(|e| panic!("{ok}: {e}"));
    }
    let mut m = good();
    exceptional(&mut m, "elsewhere");
    assert_eq!(broken(m), [Rule::EnvironmentMismatch]);
    // A stage that is not exceptional may not use the exceptional environment.
    let mut m = good();
    federated_of(&mut m).exceptional_subject = "ci-exceptional".to_owned();
    federated_of(&mut m).exceptional_environment = "publish-exceptional".to_owned();
    stage_mut(&mut m, "publish").environment = "publish-exceptional".to_owned();
    assert_eq!(broken(m), [Rule::EnvironmentMismatch]);
}

#[test]
fn an_anonymous_repository_names_no_environment() {
    let mut m = good();
    stable_repo(&mut m).push = Some(anonymous());
    stage_mut(&mut m, "publish").environment = "anything".to_owned();
    Checked::read(m).expect("an anonymous push has no environment to match");
}

#[test]
fn publish_stage_has_build_is_refused() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "publish").steps.push(build_step(
            "extra",
            "linux-x86_64",
            &["tool"],
            vec![]
        ))),
        [Rule::PublishStageHasBuild]
    );
}

#[test]
fn a_publish_stage_that_needs_no_build_is_refused() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "publish").needs = vec![]),
        [Rule::PublishStageNeedsValidatedBuild]
    );
}

#[test]
fn a_publish_stage_that_needs_two_builds_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let mut second = stage_mut(m, "build").clone();
            second.name = "build2".to_owned();
            m.stages.insert(2, second);
            stage_mut(m, "publish").needs.push("build2".to_owned());
        }),
        [Rule::PublishStageNeedsValidatedBuild]
    );
}

#[test]
fn a_publish_stage_whose_build_has_only_another_validator_is_refused() {
    assert_eq!(
        refused_after(
            |m| stage_mut(m, "build").steps[0].validations[0].validator = "fake".to_owned()
        ),
        [Rule::PublishStageNeedsValidatedBuild]
    );
    assert_eq!(
        refused_after(|m| stage_mut(m, "build").steps[0].validations.clear()),
        [Rule::PublishStageNeedsValidatedBuild]
    );
}

#[test]
fn a_publish_stage_may_need_stages_that_hold_no_build() {
    let mut m = good();
    m.stages
        .insert(0, stage("approval", Trigger::Release, &[], vec![]));
    stage_mut(&mut m, "publish")
        .needs
        .push("approval".to_owned());
    Checked::read(m).expect("one build stage is needed; others hold none");
}

#[test]
fn a_publish_chain_over_two_builds_is_refused() {
    let mut m = good();
    with_second_publish(&mut m, &["publish", "build2"]);
    assert_eq!(broken(m), [Rule::PublishChainSharesTheBuild]);
}

#[test]
fn a_publish_chain_over_one_build_is_read() {
    let mut m = good();
    with_second_publish(&mut m, &["publish", "build"]);
    Checked::read(m).expect("both stages publish the set of one build");
}

#[test]
fn a_publish_stage_of_another_platform_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let Some(Action::Publish(p)) = &mut stage_mut(m, "publish").steps[0].action else {
                panic!("a publish step");
            };
            p.platform = "linux-aarch64".to_owned();
        }),
        [Rule::PublishStageScope]
    );
}

#[test]
fn a_publish_of_an_artifact_the_build_lacks_is_refused() {
    assert_eq!(
        refused_after(|m| {
            m.artifacts.push(decl("extra", ArtifactType::File, &[]));
            let Some(Action::Publish(p)) = &mut stage_mut(m, "publish").steps[0].action else {
                panic!("a publish step");
            };
            p.artifacts.push("extra".to_owned());
        }),
        [Rule::PublishStageScope]
    );
}

#[test]
fn a_bound_above_the_maximum_is_refused() {
    assert_eq!(
        refused_after(|m| {
            let Some(Action::Publish(p)) = &mut stage_mut(m, "publish").steps[0].action else {
                panic!("a publish step");
            };
            p.attempts = 11;
        }),
        [Rule::BoundOutOfRange]
    );
    assert_eq!(
        refused_after(|m| {
            let Some(Action::Publish(p)) = &mut stage_mut(m, "publish").steps[0].action else {
                panic!("a publish step");
            };
            p.polls = 11;
        }),
        [Rule::BoundOutOfRange]
    );
    assert_eq!(
        refused_after(|m| stage_mut(m, "publish").steps[0].validations[0].wait_polls = 11),
        [Rule::BoundOutOfRange]
    );
    let mut m = good();
    let Some(Action::Publish(p)) = &mut stage_mut(&mut m, "publish").steps[0].action else {
        panic!("a publish step");
    };
    (p.attempts, p.polls) = (10, 10);
    stage_mut(&mut m, "publish").steps[0].validations[0].wait_polls = 10;
    Checked::read(m).expect("the maximum itself is allowed");
}

#[test]
fn zero_bounds_read_as_the_defaults() {
    let mut m = good();
    let Some(Action::Publish(p)) = &mut stage_mut(&mut m, "publish").steps[0].action else {
        panic!("a publish step");
    };
    assert_eq!((p.attempts, p.polls), (0, 0));
    assert_eq!(put_attempts(p), DEFAULT_PUT_ATTEMPTS);
    assert_eq!(readback_polls(p), DEFAULT_READBACK_POLLS);
    assert_eq!((DEFAULT_PUT_ATTEMPTS, DEFAULT_READBACK_POLLS), (3, 3));
    (p.attempts, p.polls) = (7, 9);
    assert_eq!((put_attempts(p), readback_polls(p)), (7, 9));
}

#[test]
fn gating_stage_without_readback_is_refused() {
    let gated = |m: &mut Machine| {
        m.stages
            .push(stage("after", Trigger::Release, &["publish"], vec![]));
    };
    let mut m = good();
    gated(&mut m);
    Checked::read(m).expect("the publish stage reads back what it published");
    assert_eq!(
        refused_after(|m| {
            gated(m);
            stage_mut(m, "publish").steps[0].validations.clear();
        }),
        [Rule::GatingStageLacksReadback]
    );
}

#[test]
fn exceptional_needs_exceptional() {
    assert_eq!(
        refused_after(|m| stage_mut(m, "build").exceptional = true),
        [Rule::ExceptionalNeedsExceptional]
    );
    assert_eq!(
        refused_after(|m| stage_mut(m, "publish").exceptional = true),
        [Rule::ExceptionalNeedsExceptional]
    );
}

#[test]
fn a_chain_over_a_stage_that_needs_no_build_is_refused_for_that_stage_only() {
    // The earlier publish stage needs no build; the chain rule has no build of its to compare.
    let mut m = good();
    with_second_publish(&mut m, &["publish", "build"]);
    stage_mut(&mut m, "publish").needs.clear();
    assert_eq!(broken(m), [Rule::PublishStageNeedsValidatedBuild]);
}

#[test]
fn a_local_set_validation_on_a_build_that_is_not_a_closure_check_is_refused() {
    // A validation of another subject, even one named for the closure check, is not the check.
    assert_eq!(
        refused_after(|m| {
            let v = &mut stage_mut(m, "build").steps[0].validations[0];
            v.subject = Subject::ChannelReadback as i32;
        }),
        [Rule::ReadbackOnBuild, Rule::PublishStageNeedsValidatedBuild]
    );
}

#[test]
fn every_artifact_type_names_its_identity() {
    // Every value of the enum, so a type added without a decision here fails this test.
    let all: Vec<ArtifactType> = (0..=64)
        .filter_map(|n| ArtifactType::try_from(n).ok())
        .collect();
    assert_eq!(
        all,
        [
            ArtifactType::Unspecified,
            ArtifactType::File,
            ArtifactType::Oci,
            ArtifactType::Conda,
            ArtifactType::Python,
            ArtifactType::Npm
        ]
    );
    assert_eq!(identity(ArtifactType::File), Some(Identity::FileBytes));
    assert_eq!(identity(ArtifactType::Npm), Some(Identity::FileBytes));
    assert_eq!(
        identity(ArtifactType::Oci),
        Some(Identity::OciManifestDigest)
    );
    // A type with no identity is not accepted by the reader.
    for kind in [ArtifactType::Conda, ArtifactType::Python] {
        assert_eq!(identity(kind), None);
        assert_eq!(
            refused_after(|m| m.artifacts.push(decl("pkg", kind, &[]))),
            [Rule::EveryArtifactTypeNamesItsIdentity],
            "{kind:?}"
        );
    }
    for kind in [ArtifactType::File, ArtifactType::Oci, ArtifactType::Npm] {
        let mut m = good();
        m.artifacts.push(decl("pkg", kind, &[]));
        Checked::read(m).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
    }
}

#[test]
fn the_views_of_a_stage_answer_from_its_steps() {
    let m = good();
    let build = m.stages[1].clone();
    let publish = m.stages[2].clone();
    assert_eq!(stage_platform(&build), Some("linux-x86_64"));
    assert_eq!(
        stage_platform(&stage("empty", Trigger::Release, &[], vec![])),
        None
    );
    assert_eq!(step_platform(&build.steps[0]), Some("linux-x86_64"));
    assert_eq!(
        step_platform(&Step {
            name: "idle".to_owned(),
            action: None,
            validations: vec![]
        }),
        None
    );
    assert_eq!(stage_environment(&publish), "publish");
    let mut named = publish.clone();
    named.environment = "chosen".to_owned();
    assert_eq!(stage_environment(&named), "chosen");
    let names: Vec<&str> = needed_builds(&m, &publish)
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["build"]);
}
