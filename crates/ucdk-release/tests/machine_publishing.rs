// SPDX-License-Identifier: Apache-2.0
//! Reading a machine, the rules of channels and push identities, the Publish and Smoke
//! actions, the places a validation may sit, the trust split between the stage that builds and
//! the stage that publishes, and the authored bounds.

mod common;

use common::*;
use ucdk_release::machine::{
    DEFAULT_PUT_ATTEMPTS, DEFAULT_READBACK_POLLS, MAX_BOUND, Rule, put_attempts, readback_polls,
};
use ucdk_release_proto::v1::{
    ArtifactType, Machine, Publish, Stage, Trigger, ValidationKind, Visibility,
};

/// A stage that only tests, to hang a rule on.
fn tester(name: &str, needs: &[&str]) -> Stage {
    stage(
        name,
        Trigger::Release,
        needs,
        vec![test_action("test", &["//release:set_test"])],
    )
}

/// A second channel with one anonymous repository.
fn mirror() -> ucdk_release_proto::v1::Channel {
    channel(
        "mirror",
        vec![repo(
            ArtifactType::File,
            "https://mirror.example.com/files",
            Visibility::Public,
            anonymous_push(),
        )],
    )
}

/// A second release build stage of the same set of artifacts, and a second publishing stage
/// that needs `needs`.
fn with_second_publish(m: &mut Machine, needs: &[&str]) {
    let mut again = stage_mut(m, "build").clone();
    again.name = "build2".to_owned();
    m.stages.push(again);
    m.stages.push(Stage {
        environment: "publish".to_owned(),
        ..stage(
            "publish2",
            Trigger::Release,
            needs,
            vec![publish_action(
                "publish",
                PLATFORM,
                &["ucdk-cli", "ucdk-lib"],
                "registry",
            )],
        )
    });
}

#[test]
fn a_channel_with_no_repository_or_shape_is_refused() {
    refuses_only(Rule::ChannelShape, |m| {
        m.channels.push(channel("empty", Vec::new()));
    });
    refuses_only(Rule::ChannelShape, |m| {
        let mut again = repo_mut(m).clone();
        again.location = "https://packages.example.com/other".to_owned();
        m.channels[0].repositories.push(again);
    });
    refuses_only(Rule::ChannelShape, |m| repo_mut(m).push = None);
}

#[test]
fn a_location_shared_by_two_repositories_is_refused() {
    // Across channels, and inside one.
    refuses_only(Rule::ChannelShape, |m| {
        let mut other = mirror();
        other.repositories[0].location = repo_mut(m).location.clone();
        m.channels.push(other);
    });
    refuses_only(Rule::ChannelShape, |m| {
        let location = repo_mut(m).location.clone();
        m.channels[0].repositories.push(repo(
            ArtifactType::Oci,
            &location,
            Visibility::Public,
            anonymous_push(),
        ));
    });
    // Compared byte for byte: a trailing slash is another location.
    accepts(|m| {
        let mut other = mirror();
        other.repositories[0].location = format!("{}/", repo_mut(m).location);
        m.channels.push(other);
    });
}

#[test]
fn a_federated_identity_without_scopes_is_refused() {
    refuses_only(Rule::FederatedShape, |m| {
        federated_mut(m).stage_scope = String::new();
    });
    refuses_only(Rule::FederatedShape, |m| {
        federated_mut(m).commit_scope = String::new();
    });
    refuses_only(Rule::FederatedShape, |m| {
        federated_mut(m).subject = String::new();
    });
    refuses_only(Rule::FederatedShape, |m| {
        federated_mut(m).audience = String::new();
    });
    // The scopes may be equal, and differ when the issuer has two.
    accepts(|m| federated_mut(m).commit_scope = "commit".to_owned());
}

#[test]
fn pinned_claims_are_compared_not_interpreted() {
    for key in ["", "sub", "aud", "exp"] {
        refuses_only(Rule::FederatedShape, |m| {
            federated_mut(m)
                .pinned_claims
                .insert(key.to_owned(), "x".to_owned());
        });
    }
    refuses_only(Rule::FederatedShape, |m| {
        federated_mut(m)
            .pinned_claims
            .insert("repository_id".to_owned(), String::new());
    });
    accepts(|m| {
        let claims = &mut federated_mut(m).pinned_claims;
        claims.insert("repository_id".to_owned(), "1234".to_owned());
        claims.insert("job_workflow_ref".to_owned(), "any text at all".to_owned());
    });
}

#[test]
fn a_private_repository_without_a_read_scope_is_refused() {
    refuses_only(Rule::FederatedShape, |m| {
        repo_mut(m).visibility = Visibility::Private as i32;
    });
    accepts(|m| {
        repo_mut(m).visibility = Visibility::Private as i32;
        federated_mut(m).read_scope = "read".to_owned();
    });
    // The scope may equal either of the other two.
    accepts(|m| {
        repo_mut(m).visibility = Visibility::Private as i32;
        federated_mut(m).read_scope = "publish".to_owned();
    });
    // An anonymous identity asks for no scope at all.
    accepts(|m| {
        repo_mut(m).visibility = Visibility::Private as i32;
        repo_mut(m).push = Some(anonymous_push());
    });
}

#[test]
fn a_read_scope_on_a_public_repository_is_refused() {
    refuses_only(Rule::FederatedShape, |m| {
        federated_mut(m).read_scope = "read".to_owned();
    });
}

#[test]
fn exceptional_environment_without_its_subject_is_refused() {
    refuses_only(Rule::ExceptionalSubject, |m| {
        federated_mut(m).exceptional_environment = "break-glass".to_owned();
    });
    refuses_only(Rule::ExceptionalSubject, |m| {
        federated_mut(m).exceptional_subject =
            "repo:example-org/ucdk:environment:break-glass".to_owned();
    });
}

#[test]
fn an_exceptional_identity_must_differ_from_the_ordinary_one() {
    refuses_only(Rule::ExceptionalSubject, |m| {
        let fed = federated_mut(m);
        fed.exceptional_subject = fed.subject.clone();
        fed.exceptional_environment = "break-glass".to_owned();
    });
    refuses_only(Rule::ExceptionalSubject, |m| {
        let fed = federated_mut(m);
        fed.exceptional_subject = "repo:example-org/ucdk:environment:break-glass".to_owned();
        fed.exceptional_environment = fed.environment.clone();
    });
    accepts(|m| {
        let fed = federated_mut(m);
        fed.exceptional_subject = "repo:example-org/ucdk:environment:break-glass".to_owned();
        fed.exceptional_environment = "break-glass".to_owned();
    });
}

#[test]
fn a_publish_of_an_artifact_the_build_lacks_is_refused() {
    refuses_only(Rule::PublishStageScope, |m| {
        build_mut(m, "build", "build").artifacts = vec!["ucdk-cli".to_owned()];
    });
}

#[test]
fn a_publish_or_smoke_naming_an_undeclared_artifact_is_refused() {
    refuses(Rule::PublishUndeclared, |m| {
        publish_mut(m, "publish", "publish")
            .artifacts
            .push("ghost".to_owned());
    });
    refuses(Rule::PublishUndeclared, |m| {
        smoke_mut(m, "publish", "smoke")
            .artifacts
            .push("ghost".to_owned());
    });
    // A channel with no repository for the type of an artifact it is asked to publish.
    refuses_only(Rule::PublishUndeclared, |m| {
        artifact_mut(m, "ucdk-lib").r#type = ArtifactType::Oci as i32;
    });
}

#[test]
fn a_validation_on_a_place_its_kind_may_not_hold_is_refused() {
    let closure = || validation("extra", ValidationKind::SetClosure, 0);
    let readback = || validation("extra", ValidationKind::ChannelReadback, 0);
    refuses(Rule::ValidationPlacement, |m| {
        action_mut(m, "publish", "smoke")
            .validations
            .push(closure());
    });
    refuses(Rule::ValidationPlacement, |m| {
        action_mut(m, "build", "build").validations.push(readback());
    });
    refuses(Rule::ValidationPlacement, |m| {
        action_mut(m, "publish", "publish")
            .validations
            .push(readback());
    });
    refuses(Rule::ValidationPlacement, |m| {
        stage_mut(m, "publish").validations.push(closure());
    });
    refuses(Rule::ValidationPlacement, |m| {
        stage_mut(m, "build").validations.push(readback());
    });
    // A closure runs in the preflight of a Publish action, and a read-back may sit on the
    // stage that publishes.
    accepts(|m| {
        action_mut(m, "publish", "publish")
            .validations
            .push(closure());
    });
    accepts(|m| {
        stage_mut(m, "publish").validations.push(readback());
    });
}

#[test]
fn a_smoke_without_readback_is_refused() {
    refuses_only(Rule::SmokeShape, |m| {
        action_mut(m, "publish", "smoke").validations.clear();
    });
    // The read-back of the stage is not the Smoke action's own.
    refuses_only(Rule::SmokeShape, |m| {
        action_mut(m, "publish", "smoke").validations.clear();
        stage_mut(m, "publish").validations.push(validation(
            "visible",
            ValidationKind::ChannelReadback,
            0,
        ));
    });
}

#[test]
fn a_smoke_of_an_unpublished_artifact_is_refused() {
    refuses_only(Rule::SmokeShape, |m| {
        publish_mut(m, "publish", "publish").artifacts = vec!["ucdk-cli".to_owned()];
    });
    refuses(Rule::SmokeShape, |m| {
        m.channels.push(mirror());
        smoke_mut(m, "publish", "smoke").channel = "mirror".to_owned();
    });
    // An action that comes before the Publish action has nothing to read.
    refuses(Rule::SmokeShape, |m| {
        stage_mut(m, "publish").actions.reverse();
    });
}

#[test]
fn environment_mismatch_is_refused() {
    refuses_only(Rule::EnvironmentMismatch, |m| {
        stage_mut(m, "publish").environment = "other".to_owned();
    });
    // An unset environment is the stage's name, which here is the identity's environment.
    accepts(|m| stage_mut(m, "publish").environment = String::new());
    refuses_only(Rule::EnvironmentMismatch, |m| {
        federated_mut(m).environment = "elsewhere".to_owned();
    });
}

#[test]
fn an_exceptional_stage_may_run_in_the_exceptional_environment() {
    let exceptional = |m: &mut Machine| {
        let fed = federated_mut(m);
        fed.exceptional_subject = "repo:example-org/ucdk:environment:break-glass".to_owned();
        fed.exceptional_environment = "break-glass".to_owned();
        stage_mut(m, "build").exceptional = true;
        stage_mut(m, "publish").exceptional = true;
    };
    accepts(|m| {
        exceptional(m);
        stage_mut(m, "publish").environment = "break-glass".to_owned();
    });
    // The ordinary environment still serves an exceptional stage.
    accepts(exceptional);
    // A stage that is not exceptional may not use the exceptional environment, and an
    // exceptional one may use neither of an unrelated name.
    refuses_only(Rule::EnvironmentMismatch, |m| {
        let fed = federated_mut(m);
        fed.exceptional_subject = "repo:example-org/ucdk:environment:break-glass".to_owned();
        fed.exceptional_environment = "break-glass".to_owned();
        stage_mut(m, "publish").environment = "break-glass".to_owned();
    });
    refuses_only(Rule::EnvironmentMismatch, |m| {
        exceptional(m);
        stage_mut(m, "publish").environment = "other".to_owned();
    });
}

#[test]
fn publish_stage_runs_code_is_refused() {
    refuses(Rule::PublishStageRunsCode, |m| {
        stage_mut(m, "publish").actions.push(build_action(
            "build",
            PLATFORM,
            &["ucdk-cli", "ucdk-lib"],
            Vec::new(),
        ));
    });
}

#[test]
fn a_publish_stage_that_holds_a_test_is_refused() {
    refuses_only(Rule::PublishStageRunsCode, |m| {
        stage_mut(m, "publish")
            .actions
            .push(test_action("test", &["//release:set_test"]));
    });
    // A Smoke action runs no code, and a later stage that needs the publishing stage may test.
    accepts(|m| m.stages.push(tester("after", &["publish"])));
}

#[test]
fn a_publish_stage_whose_build_has_no_closure_validation_is_refused() {
    refuses_only(Rule::PublishStageNeedsValidatedBuild, |m| {
        action_mut(m, "build", "build").validations.clear();
    });
    // Only a closure on the Build action satisfies the rule, never one of the stage.
    refuses(Rule::PublishStageNeedsValidatedBuild, |m| {
        action_mut(m, "build", "build").validations.clear();
        stage_mut(m, "build").validations.push(validation(
            "closure",
            ValidationKind::SetClosure,
            0,
        ));
    });
    refuses(Rule::PublishStageNeedsValidatedBuild, |m| {
        stage_mut(m, "publish").needs.clear();
    });
}

#[test]
fn a_publish_stage_that_needs_two_builds_is_refused() {
    refuses(Rule::PublishStageNeedsValidatedBuild, |m| {
        let mut again = stage_mut(m, "build").clone();
        again.name = "build2".to_owned();
        m.stages.insert(2, again);
        stage_mut(m, "publish").needs.push("build2".to_owned());
    });
    // It may need stages that hold no Build action as well.
    accepts(|m| {
        m.stages.insert(2, tester("lint", &[]));
        stage_mut(m, "publish").needs.push("lint".to_owned());
    });
}

#[test]
fn a_publish_chain_over_two_builds_is_refused() {
    refuses_only(Rule::PublishChainSharesTheBuild, |m| {
        with_second_publish(m, &["publish", "build2"]);
    });
    accepts(|m| with_second_publish(m, &["publish", "build"]));
}

#[test]
fn a_publish_stage_of_another_platform_is_refused() {
    refuses_only(Rule::PublishStageScope, |m| {
        publish_mut(m, "publish", "publish").platform = "darwin-aarch64".to_owned();
        smoke_mut(m, "publish", "smoke").platform = "darwin-aarch64".to_owned();
    });
    accepts(|m| {
        build_mut(m, "build", "build").platform = "any".to_owned();
        publish_mut(m, "publish", "publish").platform = "any".to_owned();
        smoke_mut(m, "publish", "smoke").platform = "any".to_owned();
    });
}

#[test]
fn a_bound_above_the_maximum_is_refused() {
    let over = MAX_BOUND + 1;
    refuses_only(Rule::BoundOutOfRange, |m| {
        publish_mut(m, "publish", "publish").attempts = over;
    });
    refuses_only(Rule::BoundOutOfRange, |m| {
        publish_mut(m, "publish", "publish").polls = over;
    });
    refuses_only(Rule::BoundOutOfRange, |m| {
        action_mut(m, "publish", "smoke").validations[0].wait_polls = over;
    });
    accepts(|m| {
        publish_mut(m, "publish", "publish").attempts = MAX_BOUND;
        publish_mut(m, "publish", "publish").polls = MAX_BOUND;
        action_mut(m, "publish", "smoke").validations[0].wait_polls = MAX_BOUND;
    });
}

#[test]
fn zero_bounds_read_as_the_defaults() {
    let publish = |attempts, polls| Publish {
        attempts,
        polls,
        ..Publish::default()
    };
    assert_eq!(put_attempts(&publish(0, 0)), DEFAULT_PUT_ATTEMPTS);
    assert_eq!(readback_polls(&publish(0, 0)), DEFAULT_READBACK_POLLS);
    assert_eq!(put_attempts(&publish(7, 9)), 7);
    assert_eq!(readback_polls(&publish(7, 9)), 9);
    // The example states no bound and is read.
    accepts(|_| {});
}

#[test]
fn gating_stage_without_readback_is_refused() {
    fn gated(m: &mut Machine) {
        m.stages.push(tester("after", &["publish"]));
    }
    refuses_only(Rule::GatingStageLacksReadback, |m| {
        stage_mut(m, "publish").actions.truncate(1);
        gated(m);
    });
    // A Smoke action's read-back, or a read-back on the stage, makes it a gate.
    accepts(gated);
    accepts(|m| {
        stage_mut(m, "publish").actions.truncate(1);
        stage_mut(m, "publish").validations.push(validation(
            "visible",
            ValidationKind::ChannelReadback,
            0,
        ));
        gated(m);
    });
    // A stage that nothing needs is not a gate.
    accepts(|m| stage_mut(m, "publish").actions.truncate(1));
}
