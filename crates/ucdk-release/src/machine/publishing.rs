// SPDX-License-Identifier: Apache-2.0
//! The rules about channels and the identities that push to them, the Publish and Smoke
//! actions, the places a validation may sit, and the trust split between the stage that builds
//! and the stage that holds a push identity.

use std::collections::{BTreeMap, BTreeSet};

use ucdk_release_proto::v1::action::Kind as ActionKind;
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{
    Action, Channel, ChannelKind, Federated, Machine, Repository, Stage, ValidationKind, Visibility,
};

use super::{
    Findings, MAX_BOUND, Rule, artifact_named, builds, channel_named, needed_builds, needed_names,
    publishes, repeated, smokes, stage_environment, stage_named, stage_platform, tests,
    validations,
};

pub(super) fn check(m: &Machine, f: &mut Findings) {
    check_channels(m, f);
    for stage in &m.stages {
        check_actions(m, stage, f);
        check_validations(stage, f);
    }
    check_trust(m, f);
}

fn check_channels(m: &Machine, f: &mut Findings) {
    let mut locations: BTreeMap<&str, usize> = BTreeMap::new();
    for channel in &m.channels {
        if ChannelKind::try_from(channel.kind) == Ok(ChannelKind::Unspecified) {
            f.add(
                Rule::ChannelShape,
                format!("channel {} has no kind", channel.name),
            );
        }
        if channel.repositories.is_empty() {
            f.add(
                Rule::ChannelShape,
                format!("channel {} has no repository", channel.name),
            );
        }
        for kind in repeated(channel.repositories.iter().map(|r| type_text(r.r#type))) {
            f.add(
                Rule::ChannelShape,
                format!(
                    "channel {} has two repositories of type {kind}",
                    channel.name
                ),
            );
        }
        for repo in &channel.repositories {
            *locations.entry(&repo.location).or_default() += 1;
            check_push(channel, repo, f);
        }
    }
    for (location, count) in locations {
        if count > 1 {
            f.add(
                Rule::ChannelShape,
                format!("the location {location:?} is shared by {count} repositories"),
            );
        }
    }
}

fn type_text(raw: i32) -> &'static str {
    ucdk_release_proto::v1::ArtifactType::try_from(raw).map_or("UNKNOWN", |t| t.as_str_name())
}

fn check_push(channel: &Channel, repo: &Repository, f: &mut Findings) {
    let whose = format!(
        "the repository {} of channel {}",
        repo.location, channel.name
    );
    let fed = match repo.push.as_ref().and_then(|p| p.kind.as_ref()) {
        None => {
            f.add(Rule::ChannelShape, format!("{whose} has no push identity"));
            return;
        }
        Some(Kind::Anonymous(_)) => return,
        Some(Kind::Federated(fed)) => fed,
    };
    let mut exceptional = |why: &str| f.add(Rule::ExceptionalSubject, format!("{whose} {why}"));
    if !fed.exceptional_subject.is_empty() && fed.exceptional_subject == fed.subject {
        exceptional("has an exceptional subject equal to its subject");
    }
    if !fed.exceptional_environment.is_empty() && fed.exceptional_environment == fed.environment {
        exceptional("has an exceptional environment equal to its environment");
    }
    if fed.exceptional_subject.is_empty() != fed.exceptional_environment.is_empty() {
        exceptional("sets only one of its exceptional subject and exceptional environment");
    }
    check_federated(&whose, repo, fed, f);
}

fn check_federated(whose: &str, repo: &Repository, fed: &Federated, f: &mut Findings) {
    let mut refuse = |why: &str| f.add(Rule::FederatedShape, format!("{whose} {why}"));
    for (field, value) in [
        ("subject", &fed.subject),
        ("audience", &fed.audience),
        ("stage_scope", &fed.stage_scope),
        ("commit_scope", &fed.commit_scope),
    ] {
        if value.is_empty() {
            refuse(&format!("has an empty {field}"));
        }
    }
    for (key, value) in &fed.pinned_claims {
        if key.is_empty() || matches!(key.as_str(), "sub" | "aud" | "exp") {
            refuse(&format!(
                "pins the claim {key:?}, which is empty or has a field of its own"
            ));
        }
        if value.is_empty() {
            refuse(&format!("pins the claim {key:?} to an empty value"));
        }
    }
    if repo.visibility == Visibility::Private as i32 && fed.read_scope.is_empty() {
        refuse("is PRIVATE and has no read_scope, so a read of it has no scope to ask for");
    }
    if repo.visibility == Visibility::Public as i32 && !fed.read_scope.is_empty() {
        refuse("is PUBLIC and has a read_scope, and its reads are anonymous");
    }
}

fn check_actions(m: &Machine, stage: &Stage, f: &mut Findings) {
    // The (channel, artifact) pairs the Publish actions before the current action published.
    let mut published: BTreeSet<(&str, &str)> = BTreeSet::new();
    for action in &stage.actions {
        match &action.kind {
            Some(ActionKind::Publish(p)) => {
                for (what, value) in [("attempts", p.attempts), ("polls", p.polls)] {
                    if value > MAX_BOUND {
                        f.add(
                            Rule::BoundOutOfRange,
                            format!(
                                "action {} has {what} {value}, above {MAX_BOUND}",
                                action.name
                            ),
                        );
                    }
                }
                check_channel_and_artifacts(m, action, &p.channel, &p.artifacts, f);
                published.extend(p.artifacts.iter().map(|a| (p.channel.as_str(), a.as_str())));
            }
            Some(ActionKind::Smoke(s)) => {
                check_channel_and_artifacts(m, action, &s.channel, &s.artifacts, f);
                check_smoke(action, &s.channel, &s.artifacts, &published, f);
            }
            _ => {}
        }
    }
}

/// The channel an action names is declared, and has a repository for the type of each artifact
/// it names.
fn check_channel_and_artifacts(
    m: &Machine,
    action: &Action,
    channel_name: &str,
    artifacts: &[String],
    f: &mut Findings,
) {
    let channel = channel_named(m, channel_name);
    if channel.is_none() {
        let declared: Vec<&str> = m.channels.iter().map(|c| c.name.as_str()).collect();
        f.add(
            Rule::UnknownChannel,
            format!(
                "action {} names the channel {channel_name:?}, and the declared channels are [{}]",
                action.name,
                declared.join(", ")
            ),
        );
    }
    for name in artifacts {
        match artifact_named(m, name) {
            None => f.add(
                Rule::PublishUndeclared,
                format!("action {} names {name}, which is not declared", action.name),
            ),
            Some(artifact) => {
                if channel
                    .is_some_and(|c| c.repositories.iter().all(|r| r.r#type != artifact.r#type))
                {
                    f.add(
                        Rule::PublishUndeclared,
                        format!("channel {channel_name} has no repository for the type of {name}"),
                    );
                }
            }
        }
    }
}

fn check_smoke(
    action: &Action,
    channel: &str,
    artifacts: &[String],
    published: &BTreeSet<(&str, &str)>,
    f: &mut Findings,
) {
    if !action
        .validations
        .iter()
        .any(|v| v.kind == ValidationKind::ChannelReadback as i32)
    {
        f.add(
            Rule::SmokeShape,
            format!(
                "the smoke action {} has no CHANNEL_READBACK validation",
                action.name
            ),
        );
    }
    if !published.iter().any(|(c, _)| *c == channel) {
        f.add(
            Rule::SmokeShape,
            format!(
                "the smoke action {} reads the channel {channel:?}, which no earlier Publish action of its stage published to",
                action.name
            ),
        );
        return;
    }
    for name in artifacts
        .iter()
        .filter(|a| !published.contains(&(channel, a.as_str())))
    {
        f.add(
            Rule::SmokeShape,
            format!(
                "the smoke action {} reads {name}, which no earlier Publish action of its stage published to {channel}",
                action.name
            ),
        );
    }
}

/// Which holder may carry which kind, and the bound of a read-back. A Test action holds none
/// (`machine.test_shape` refuses its validations as well, as part of the shape of a test).
fn check_validations(stage: &Stage, f: &mut Findings) {
    let closure = ValidationKind::SetClosure as i32;
    let readback = ValidationKind::ChannelReadback as i32;
    for action in &stage.actions {
        let (holder, allowed) = match &action.kind {
            Some(ActionKind::Build(_)) => ("a Build action", closure),
            Some(ActionKind::Publish(_)) => ("a Publish action", closure),
            Some(ActionKind::Smoke(_)) => ("a Smoke action", readback),
            Some(ActionKind::Test(_)) => ("a Test action", 0),
            None => continue,
        };
        for v in &action.validations {
            if is_kind(v.kind) && v.kind != allowed {
                f.add(
                    Rule::ValidationPlacement,
                    format!(
                        "validation {} is a {} and sits on {holder}, which holds {}",
                        v.name,
                        kind_text(v.kind),
                        if allowed == 0 {
                            "none".to_owned()
                        } else {
                            format!("only {}", kind_text(allowed))
                        }
                    ),
                );
            }
        }
    }
    let publishing = publishes(stage).next().is_some();
    for v in &stage.validations {
        let ok = match v.kind {
            k if k == readback => publishing,
            k if k == closure => false,
            _ => true,
        };
        if !ok {
            f.add(
                Rule::ValidationPlacement,
                format!(
                    "validation {} is a {} and sits on stage {}, {}",
                    v.name,
                    kind_text(v.kind),
                    stage.name,
                    if v.kind == closure {
                        "and a stage holds no closure check"
                    } else {
                        "which holds no Publish action"
                    }
                ),
            );
        }
    }
    for v in validations(stage) {
        if v.wait_polls > MAX_BOUND {
            f.add(
                Rule::BoundOutOfRange,
                format!(
                    "validation {} has wait_polls {}, above {MAX_BOUND}",
                    v.name, v.wait_polls
                ),
            );
        }
    }
}

fn is_kind(raw: i32) -> bool {
    matches!(
        ValidationKind::try_from(raw),
        Ok(ValidationKind::SetClosure | ValidationKind::ChannelReadback)
    )
}

fn kind_text(raw: i32) -> &'static str {
    ValidationKind::try_from(raw).map_or("UNKNOWN", |k| k.as_str_name())
}

fn check_trust(m: &Machine, f: &mut Findings) {
    let needed = needed_names(m);
    for stage in m.stages.iter().filter(|s| publishes(s).next().is_some()) {
        let name = &stage.name;
        if builds(stage).next().is_some() || tests(stage).next().is_some() {
            f.add(
                Rule::PublishStageRunsCode,
                format!(
                    "stage {name} holds a Publish action together with a Build or a Test action"
                ),
            );
        }
        let have = needed_builds(m, stage);
        match have.as_slice() {
            [build_stage] => check_against_build(stage, build_stage, f),
            _ => f.add(
                Rule::PublishStageNeedsValidatedBuild,
                format!("stage {name} needs {} stages that hold a Build action, and it must need exactly one", have.len()),
            ),
        }
        for earlier in stage
            .needs
            .iter()
            .filter_map(|n| stage_named(m, n))
            .filter(|s| publishes(s).next().is_some())
        {
            let theirs = needed_builds(m, earlier);
            if let ([mine], [other]) = (have.as_slice(), theirs.as_slice())
                && mine.name != other.name
            {
                f.add(
                    Rule::PublishChainSharesTheBuild,
                    format!(
                        "stage {name} publishes the set of {} after {} published the set of {}",
                        mine.name, earlier.name, other.name
                    ),
                );
            }
        }
        if needed.contains(name.as_str())
            && !validations(stage).any(|v| v.kind == ValidationKind::ChannelReadback as i32)
        {
            f.add(
                Rule::GatingStageLacksReadback,
                format!("stage {name} is needed by another stage and reads nothing back"),
            );
        }
        check_environment(m, stage, f);
    }
}

fn check_against_build(stage: &Stage, build_stage: &Stage, f: &mut Findings) {
    let closed = builds(build_stage).any(|(action, _)| {
        action
            .validations
            .iter()
            .any(|v| v.kind == ValidationKind::SetClosure as i32)
    });
    if !closed {
        f.add(
            Rule::PublishStageNeedsValidatedBuild,
            format!(
                "stage {} publishes the set of stage {}, whose Build action has no SET_CLOSURE validation",
                stage.name, build_stage.name
            ),
        );
    }
    let declared: BTreeSet<&str> = builds(build_stage)
        .flat_map(|(_, b)| b.artifacts.iter().map(String::as_str))
        .collect();
    let platform = stage_platform(build_stage);
    let named = publishes(stage)
        .map(|(a, p)| (a, p.platform.as_str(), &p.artifacts))
        .chain(smokes(stage).map(|(a, s)| (a, s.platform.as_str(), &s.artifacts)));
    for (action, action_platform, artifacts) in named {
        if Some(action_platform) != platform {
            f.add(
                Rule::PublishStageScope,
                format!(
                    "action {} is for {action_platform:?}, and stage {} builds for {:?}",
                    action.name,
                    build_stage.name,
                    platform.unwrap_or("")
                ),
            );
        }
        for name in artifacts.iter().filter(|n| !declared.contains(n.as_str())) {
            f.add(
                Rule::PublishStageScope,
                format!(
                    "action {} names {name}, which stage {} does not build",
                    action.name, build_stage.name
                ),
            );
        }
    }
}

fn check_environment(m: &Machine, stage: &Stage, f: &mut Findings) {
    let env = stage_environment(stage);
    for (action, publish) in publishes(stage) {
        let Some(channel) = channel_named(m, &publish.channel) else {
            continue;
        };
        let kinds: BTreeSet<i32> = publish
            .artifacts
            .iter()
            .filter_map(|n| artifact_named(m, n))
            .map(|a| a.r#type)
            .collect();
        for repo in channel
            .repositories
            .iter()
            .filter(|r| kinds.contains(&r.r#type))
        {
            let Some(Kind::Federated(fed)) = repo.push.as_ref().and_then(|p| p.kind.as_ref())
            else {
                continue;
            };
            let allowed =
                env == fed.environment || (stage.exceptional && env == fed.exceptional_environment);
            if !allowed {
                f.add(
                    Rule::EnvironmentMismatch,
                    format!("action {} publishes to {}, whose identity names environment {:?}, and stage {} runs in {env:?}", action.name, repo.location, fed.environment, stage.name),
                );
            }
        }
    }
}
