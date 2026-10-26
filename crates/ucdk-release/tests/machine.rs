// SPDX-License-Identifier: Apache-2.0
//! Reading a machine, the rules of names, enums, the stage graph, actions and duplicates: one
//! test per rule id, each refusing a machine that breaks that rule, and the positive cases that
//! keep a rule from refusing too much. The rules of the build system and of publishing are in
//! `machine_building.rs` and `machine_publishing.rs`.

mod common;

use std::collections::BTreeSet;

use common::*;
use ucdk_release::machine::{
    Checked, Identity, ReadError, Rule, builds, identity, needed_builds, publishes,
    stage_environment, stage_platform, validations,
};
use ucdk_release_proto::v1::{
    ArtifactType, ChannelKind, Machine, Reason, Trigger, ValidationKind, Visibility,
};

/// The rule ids of the design page (`machine.*`, section 2), held here as data so that the code
/// and the page cannot drift: the page is not part of this change's base, so the list is copied
/// from it and is compared with `Rule::ALL`.
const PAGE_RULE_IDS: [&str; 39] = [
    "machine.schema_version",
    "machine.bad_name",
    "machine.unknown_enum",
    "machine.unknown_stage",
    "machine.stage_cycle",
    "machine.action_without_kind",
    "machine.stage_without_actions",
    "machine.empty_artifact_list",
    "machine.duplicate_name",
    "machine.unknown_channel",
    "machine.build_system_required",
    "machine.build_system_shape",
    "machine.reserved_flag",
    "machine.env_key",
    "machine.placeholder",
    "machine.target_shape",
    "machine.artifact_target_missing",
    "machine.build_undeclared",
    "machine.build_not_closed",
    "machine.labels_on_non_file",
    "machine.test_shape",
    "machine.change_check_publishes",
    "machine.stage_platform",
    "machine.artifact_cycle",
    "machine.channel_shape",
    "machine.exceptional_subject",
    "machine.federated_shape",
    "machine.publish_undeclared",
    "machine.validation_placement",
    "machine.smoke_shape",
    "machine.environment_mismatch",
    "machine.publish_stage_runs_code",
    "machine.publish_stage_needs_validated_build",
    "machine.publish_chain_shares_the_build",
    "machine.publish_stage_scope",
    "machine.bound_out_of_range",
    "machine.gating_stage_lacks_readback",
    "machine.exceptional_needs_exceptional",
    "machine.every_artifact_type_names_its_identity",
];

#[test]
fn the_rule_ids_of_the_code_and_of_this_page_are_the_same_set() {
    let code: BTreeSet<&str> = Rule::ALL.iter().map(|r| r.id()).collect();
    let page: BTreeSet<&str> = PAGE_RULE_IDS.iter().copied().collect();
    assert_eq!(code.len(), Rule::ALL.len(), "a rule id is used twice");
    assert_eq!(
        page.len(),
        PAGE_RULE_IDS.len(),
        "the page lists an id twice"
    );
    let only_code: Vec<_> = code.difference(&page).collect();
    let only_page: Vec<_> = page.difference(&code).collect();
    assert!(only_code.is_empty(), "ids the page lacks: {only_code:?}");
    assert!(only_page.is_empty(), "ids the code lacks: {only_page:?}");
    assert!(code.iter().all(|id| id.starts_with("machine.")));
}

#[test]
fn the_worked_example_of_1_2_passes_every_rule() {
    let checked = Checked::read(worked_example()).expect("the first machine satisfies every rule");
    let machine = checked.machine();
    assert_eq!(machine.name, "ucdk");
    let stages: Vec<(&str, Trigger)> = machine
        .stages
        .iter()
        .map(|s| {
            (
                s.name.as_str(),
                Trigger::try_from(s.trigger).expect("trigger"),
            )
        })
        .collect();
    assert_eq!(
        stages,
        [
            ("check", Trigger::ChangeCheck),
            ("build", Trigger::Release),
            ("publish", Trigger::Release)
        ]
    );
    // The facts the page states about it: a bare binary with a PATH to be found on, a target on
    // every artifact, a closure on the build action, one platform shared by the build and the
    // publishing stage, and the environment of the push identity on the publishing stage.
    let system = machine.build_system.as_ref().expect("a build system");
    assert!(system.env.contains_key("PATH"));
    assert!(machine.artifacts.iter().all(|a| a.target.is_some()));
    let build = checked.stage("build").expect("build");
    let publish = checked.stage("publish").expect("publish");
    assert_eq!(stage_platform(build), stage_platform(publish));
    assert_eq!(stage_environment(publish), "publish");
    assert!(
        builds(build).any(|(a, _)| a.validations.iter().any(|v| v.name == "closure")),
        "the build action carries the closure"
    );
    assert_eq!(
        validations(publish)
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["visible"]
    );
    assert_eq!(needed_builds(machine, publish).len(), 1);
}

#[test]
fn the_good_machine_is_read_and_its_views_answer() {
    let checked = Checked::read(good()).expect("the machine satisfies every rule");
    assert_eq!(checked.stage("build").expect("stage").name, "build");
    assert!(checked.stage("ghost").is_none());
    assert_eq!(
        checked.artifact("ucdk-cli").expect("artifact").name,
        "ucdk-cli"
    );
    assert!(checked.artifact("ghost").is_none());
    assert_eq!(
        checked.channel("registry").expect("channel").name,
        "registry"
    );
    assert!(checked.channel("ghost").is_none());
    let check = checked.stage("check").expect("check");
    assert_eq!(stage_environment(check), "check");
    assert!(publishes(check).next().is_none());
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
fn a_schema_version_below_1_is_refused() {
    for version in [0, -1] {
        refuses_only(Rule::SchemaVersion, |m| m.schema_version = version);
    }
    let err = Checked::read(Machine::default()).expect_err("an empty machine");
    assert_eq!(err.reason(), Reason::MachineFormat);
    assert!(err.rules().contains(&Rule::SchemaVersion));
}

#[test]
fn a_name_outside_the_grammar_is_refused() {
    refuses(Rule::BadName, |m| m.name = "Ucdk".to_owned());
    refuses(Rule::BadName, |m| m.name = String::new());
    for bad in [
        "Check",
        "9check",
        "check-",
        "check_2",
        "ché",
        &"a".repeat(64),
    ] {
        refuses_only(Rule::BadName, |m| {
            stage_mut(m, "check").name = bad.to_owned();
        });
    }
    refuses_only(Rule::BadName, |m| {
        action_mut(m, "check", "build").name = "Build".to_owned();
    });
    refuses_only(Rule::BadName, |m| {
        action_mut(m, "build", "build").validations[0].name = "Closure".to_owned();
    });
    refuses_only(Rule::BadName, |m| {
        action_mut(m, "publish", "smoke").validations[0].name = "visible_2".to_owned();
    });
    refuses_only(Rule::BadName, |m| {
        m.artifacts.push(file_artifact("Bad Name", "//release:x"));
    });
    refuses_only(Rule::BadName, |m| {
        m.channels.push(channel(
            "Mirror",
            vec![repo(
                ArtifactType::File,
                "https://mirror.example.com/files",
                Visibility::Public,
                anonymous_push(),
            )],
        ));
    });
    for platform in [
        "linux",
        "Linux-x86_64",
        "linux-",
        "-x86_64",
        "linux-x86-64",
        "",
    ] {
        refuses_only(Rule::BadName, |m| {
            build_mut(m, "check", "build").platform = platform.to_owned();
        });
    }
}

#[test]
fn names_inside_the_grammar_are_accepted() {
    // An artifact may hold an underscore, a stage may be 63 bytes, a platform may be `any`.
    accepts(|m| m.artifacts.push(file_artifact("tool_2", "//release:x")));
    accepts(|m| {
        m.stages.push(stage(
            &"a".repeat(63),
            Trigger::Release,
            &[],
            vec![test_action("test", &["//release:set_test"])],
        ));
    });
    accepts(|m| build_mut(m, "check", "build").platform = "any".to_owned());
    accepts(|m| build_mut(m, "check", "build").platform = "darwin-aarch64".to_owned());
}

#[test]
fn an_unknown_enum_value_is_refused() {
    refuses(Rule::UnknownEnum, |m| system_mut(m).kind = 9);
    for trigger in [0, 9] {
        refuses(Rule::UnknownEnum, |m| {
            stage_mut(m, "check").trigger = trigger;
        });
    }
    for kind in [0, 9] {
        refuses(Rule::UnknownEnum, |m| {
            action_mut(m, "build", "build").validations[0].kind = kind;
        });
        refuses(Rule::UnknownEnum, |m| {
            action_mut(m, "publish", "smoke").validations[0].kind = kind;
        });
        refuses(Rule::UnknownEnum, |m| {
            artifact_mut(m, "ucdk-lib").r#type = kind;
        });
        refuses(Rule::UnknownEnum, |m| repo_mut(m).r#type = kind);
        refuses(Rule::UnknownEnum, |m| repo_mut(m).visibility = kind);
    }
    refuses(Rule::UnknownEnum, |m| m.channels[0].kind = 9);
}

#[test]
fn a_need_naming_no_stage_is_refused() {
    refuses(Rule::UnknownStage, |m| {
        stage_mut(m, "publish").needs.push("ghost".to_owned());
    });
}

#[test]
fn a_stage_needing_itself_or_a_later_stage_is_refused() {
    refuses(Rule::StageCycle, |m| {
        stage_mut(m, "build").needs.push("build".to_owned());
    });
    refuses(Rule::StageCycle, |m| {
        stage_mut(m, "build").needs.push("publish".to_owned());
    });
    // A stage may need any stage declared before it.
    accepts(|m| {
        m.stages.push(stage(
            "verify",
            Trigger::Release,
            &["early", "build"],
            vec![test_action("test", &["//release:set_test"])],
        ));
        m.stages.insert(
            0,
            stage(
                "early",
                Trigger::Release,
                &[],
                vec![test_action("test", &["//release:set_test"])],
            ),
        );
    });
}

#[test]
fn an_action_with_no_kind_is_refused() {
    refuses_only(Rule::ActionWithoutKind, |m| {
        action_mut(m, "check", "build").kind = None;
    });
}

#[test]
fn a_stage_with_no_actions_is_refused() {
    refuses_only(Rule::StageWithoutActions, |m| {
        stage_mut(m, "check").actions.clear();
    });
}

#[test]
fn an_action_with_an_empty_artifact_list_is_refused() {
    refuses_only(Rule::EmptyArtifactList, |m| {
        build_mut(m, "check", "build").artifacts.clear();
    });
    refuses(Rule::EmptyArtifactList, |m| {
        publish_mut(m, "publish", "publish").artifacts.clear();
    });
    refuses(Rule::EmptyArtifactList, |m| {
        smoke_mut(m, "publish", "smoke").artifacts.clear();
    });
}

#[test]
fn duplicate_action_name_is_refused() {
    refuses_only(Rule::DuplicateName, |m| {
        action_mut(m, "build", "test").name = "build".to_owned();
    });
}

#[test]
fn duplicate_channel_name_is_refused() {
    refuses_only(Rule::DuplicateName, |m| {
        let mut again = m.channels[0].clone();
        again.repositories[0].location = "https://mirror.example.com/files".to_owned();
        m.channels.push(again);
    });
}

#[test]
fn a_stage_artifact_or_validation_name_used_twice_is_refused() {
    refuses_only(Rule::DuplicateName, |m| {
        let again = stage_mut(m, "check").clone();
        m.stages.push(again);
    });
    refuses_only(Rule::DuplicateName, |m| {
        let again = m.artifacts[0].clone();
        m.artifacts.push(again);
    });
    // Action-level and stage-level validations share one namespace.
    refuses_only(Rule::DuplicateName, |m| {
        stage_mut(m, "publish").validations.push(validation(
            "visible",
            ValidationKind::ChannelReadback,
            0,
        ));
    });
    // An action and a validation of one name do not clash.
    accepts(|m| {
        action_mut(m, "build", "build").validations[0].name = "test".to_owned();
    });
}

#[test]
fn a_publish_or_smoke_naming_an_undeclared_channel_is_refused() {
    refuses(Rule::UnknownChannel, |m| {
        publish_mut(m, "publish", "publish").channel = "ghost".to_owned();
    });
    refuses(Rule::UnknownChannel, |m| {
        smoke_mut(m, "publish", "smoke").channel = "ghost".to_owned();
    });
    let mut m = good();
    publish_mut(&mut m, "publish", "publish").channel = "ghost".to_owned();
    let message = Checked::read(m).expect_err("refused").to_string();
    assert!(
        message.contains("registry"),
        "the declared channels are listed: {message}"
    );
}

#[test]
fn a_change_check_stage_that_publishes_or_has_needs_is_refused() {
    let release = || test_action("test", &["//release:set_test"]);
    refuses(Rule::ChangeCheckPublishes, |m| {
        stage_mut(m, "check").actions.push(publish_action(
            "publish",
            PLATFORM,
            &["ucdk-cli"],
            "registry",
        ));
    });
    refuses(Rule::ChangeCheckPublishes, |m| {
        stage_mut(m, "check").actions.push(smoke_action(
            "smoke",
            PLATFORM,
            &["ucdk-cli"],
            "registry",
            vec![validation("seen", ValidationKind::ChannelReadback, 0)],
        ));
    });
    refuses_only(Rule::ChangeCheckPublishes, |m| {
        m.stages.push(stage(
            "gate",
            Trigger::ChangeCheck,
            &["build"],
            vec![release()],
        ));
    });
    refuses_only(Rule::ChangeCheckPublishes, |m| {
        m.stages.push(stage(
            "after",
            Trigger::Release,
            &["check"],
            vec![release()],
        ));
    });
}

#[test]
fn change_check_with_environment_is_refused() {
    refuses_only(Rule::ChangeCheckPublishes, |m| {
        stage_mut(m, "check").environment = "ci".to_owned();
    });
}

#[test]
fn change_check_that_is_exceptional_is_refused() {
    refuses_only(Rule::ChangeCheckPublishes, |m| {
        stage_mut(m, "check").exceptional = true;
    });
}

#[test]
fn a_stage_with_two_platforms_is_refused() {
    refuses(Rule::StagePlatform, |m| {
        smoke_mut(m, "publish", "smoke").platform = "darwin-aarch64".to_owned();
    });
}

#[test]
fn a_stage_with_two_builds_is_refused() {
    refuses_only(Rule::StagePlatform, |m| {
        stage_mut(m, "check").actions.push(build_action(
            "again",
            PLATFORM,
            &["ucdk-cli", "ucdk-lib"],
            Vec::new(),
        ));
    });
}

#[test]
fn an_artifact_cycle_or_a_dangling_need_is_refused() {
    refuses_only(Rule::ArtifactCycle, |m| {
        artifact_mut(m, "ucdk-cli")
            .needs
            .push("ucdk-lib".to_owned());
        artifact_mut(m, "ucdk-lib")
            .needs
            .push("ucdk-cli".to_owned());
    });
    refuses_only(Rule::ArtifactCycle, |m| {
        artifact_mut(m, "ucdk-cli").needs.push("ghost".to_owned());
    });
    refuses_only(Rule::ArtifactCycle, |m| {
        artifact_mut(m, "ucdk-cli")
            .needs
            .push("ucdk-cli".to_owned());
    });
    // A chain is not a cycle.
    accepts(|m| {
        artifact_mut(m, "ucdk-cli")
            .needs
            .push("ucdk-lib".to_owned());
    });
}

#[test]
fn exceptional_needs_exceptional() {
    refuses_only(Rule::ExceptionalNeedsExceptional, |m| {
        stage_mut(m, "build").exceptional = true;
    });
    refuses_only(Rule::ExceptionalNeedsExceptional, |m| {
        stage_mut(m, "publish").exceptional = true;
    });
    accepts(|m| {
        stage_mut(m, "build").exceptional = true;
        stage_mut(m, "publish").exceptional = true;
    });
}

#[test]
fn every_artifact_type_names_its_identity() {
    assert_eq!(identity(ArtifactType::File), Some(Identity::FileBytes));
    assert_eq!(
        identity(ArtifactType::Oci),
        Some(Identity::OciManifestDigest)
    );
    for kind in [ArtifactType::Conda, ArtifactType::Python] {
        assert_eq!(identity(kind), None, "{kind:?}");
        refuses_only(Rule::EveryArtifactTypeNamesItsIdentity, |m| {
            artifact_mut(m, "ucdk-lib").r#type = kind as i32;
            m.channels[0].repositories.push(repo(
                kind,
                "https://packages.example.com/other",
                Visibility::Public,
                anonymous_push(),
            ));
        });
    }
    // An image is accepted, with its repository.
    accepts(|m| {
        artifact_mut(m, "ucdk-lib").r#type = ArtifactType::Oci as i32;
        m.channels[0].repositories.push(repo(
            ArtifactType::Oci,
            "https://packages.example.com/images",
            Visibility::Public,
            anonymous_push(),
        ));
    });
}

#[test]
fn a_channel_without_a_kind_is_refused() {
    refuses_only(Rule::ChannelShape, |m| {
        m.channels[0].kind = ChannelKind::Unspecified as i32;
    });
}
