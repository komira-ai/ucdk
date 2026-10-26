// SPDX-License-Identifier: Apache-2.0
//! Reading a machine: every rule a declared machine must satisfy, each refusal named by a
//! stable rule id (`machine.<name>`), so a test and a caller never parse a message.
//!
//! [`Checked::read`] is the only way to a [`Checked`] machine, so a value of that type has
//! passed every rule here. The rules are written as the condition that is refused.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ucdk_core::order;
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::step::Action;
use ucdk_release_proto::v1::{
    ArtifactDecl, ArtifactType, Build, Channel, Machine, Publish, Reason, Repository, Stage, Step,
    Subject, Trigger, Validation, Visibility,
};

/// The schema version this reader understands.
pub const SCHEMA_VERSION: i32 = 1;
/// The puts per coordinate a publish step may use when it states none.
pub const DEFAULT_PUT_ATTEMPTS: u32 = 3;
/// The read-back polls per coordinate a publish step may use when it states none.
pub const DEFAULT_READBACK_POLLS: u32 = 3;
/// The greatest value of any authored bound.
pub const MAX_BOUND: u32 = 10;
/// The validator kind a build stage must carry on its set before a stage publishes it.
pub const SET_CLOSURE: &str = "set-closure";

/// The rules of reading a machine. Each is one refused condition and has one stable id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    SchemaVersion,
    BadName,
    UnknownEnum,
    StepWithoutAction,
    DuplicateName,
    UnknownStage,
    StageCycle,
    UnknownChannel,
    ChangeCheckPublishes,
    StagePlatform,
    ArtifactCycle,
    ChannelShape,
    ExceptionalSubject,
    BuildUndeclared,
    PublishUndeclared,
    ReadbackOnBuild,
    LocalSetWithoutBuild,
    EnvironmentMismatch,
    PublishStageHasBuild,
    PublishStageNeedsValidatedBuild,
    PublishChainSharesTheBuild,
    PublishStageScope,
    BoundOutOfRange,
    GatingStageLacksReadback,
    ExceptionalNeedsExceptional,
    EveryArtifactTypeNamesItsIdentity,
}

impl Rule {
    /// Every rule, in the order they are listed here.
    pub const ALL: [Rule; 26] = [
        Rule::SchemaVersion,
        Rule::BadName,
        Rule::UnknownEnum,
        Rule::StepWithoutAction,
        Rule::DuplicateName,
        Rule::UnknownStage,
        Rule::StageCycle,
        Rule::UnknownChannel,
        Rule::ChangeCheckPublishes,
        Rule::StagePlatform,
        Rule::ArtifactCycle,
        Rule::ChannelShape,
        Rule::ExceptionalSubject,
        Rule::BuildUndeclared,
        Rule::PublishUndeclared,
        Rule::ReadbackOnBuild,
        Rule::LocalSetWithoutBuild,
        Rule::EnvironmentMismatch,
        Rule::PublishStageHasBuild,
        Rule::PublishStageNeedsValidatedBuild,
        Rule::PublishChainSharesTheBuild,
        Rule::PublishStageScope,
        Rule::BoundOutOfRange,
        Rule::GatingStageLacksReadback,
        Rule::ExceptionalNeedsExceptional,
        Rule::EveryArtifactTypeNamesItsIdentity,
    ];

    /// The stable id of the rule, `machine.<name>`.
    pub fn id(self) -> &'static str {
        match self {
            Rule::SchemaVersion => "machine.schema_version",
            Rule::BadName => "machine.bad_name",
            Rule::UnknownEnum => "machine.unknown_enum",
            Rule::StepWithoutAction => "machine.step_without_action",
            Rule::DuplicateName => "machine.duplicate_name",
            Rule::UnknownStage => "machine.unknown_stage",
            Rule::StageCycle => "machine.stage_cycle",
            Rule::UnknownChannel => "machine.unknown_channel",
            Rule::ChangeCheckPublishes => "machine.change_check_publishes",
            Rule::StagePlatform => "machine.stage_platform",
            Rule::ArtifactCycle => "machine.artifact_cycle",
            Rule::ChannelShape => "machine.channel_shape",
            Rule::ExceptionalSubject => "machine.exceptional_subject",
            Rule::BuildUndeclared => "machine.build_undeclared",
            Rule::PublishUndeclared => "machine.publish_undeclared",
            Rule::ReadbackOnBuild => "machine.readback_on_build",
            Rule::LocalSetWithoutBuild => "machine.local_set_without_build",
            Rule::EnvironmentMismatch => "machine.environment_mismatch",
            Rule::PublishStageHasBuild => "machine.publish_stage_has_build",
            Rule::PublishStageNeedsValidatedBuild => "machine.publish_stage_needs_validated_build",
            Rule::PublishChainSharesTheBuild => "machine.publish_chain_shares_the_build",
            Rule::PublishStageScope => "machine.publish_stage_scope",
            Rule::BoundOutOfRange => "machine.bound_out_of_range",
            Rule::GatingStageLacksReadback => "machine.gating_stage_lacks_readback",
            Rule::ExceptionalNeedsExceptional => "machine.exceptional_needs_exceptional",
            Rule::EveryArtifactTypeNamesItsIdentity => {
                "machine.every_artifact_type_names_its_identity"
            }
        }
    }
}

/// One refused condition: the rule and a sentence for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub rule: Rule,
    pub message: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.rule.id(), self.message)
    }
}

/// Why a machine was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The machine is of a newer schema than this reader knows.
    Version { found: i32 },
    /// The machine breaks one or more rules, in the order they were found.
    Refused(Vec<Refusal>),
}

impl ReadError {
    /// The reason a verdict carries for this refusal.
    pub fn reason(&self) -> Reason {
        match self {
            ReadError::Version { .. } => Reason::MachineVersion,
            ReadError::Refused(_) => Reason::MachineFormat,
        }
    }

    /// The rules broken, each once, in rule order.
    pub fn rules(&self) -> Vec<Rule> {
        match self {
            ReadError::Version { .. } => Vec::new(),
            ReadError::Refused(found) => found
                .iter()
                .map(|r| r.rule)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Version { found } => write!(
                f,
                "the machine is schema version {found}, and this reader knows {SCHEMA_VERSION}"
            ),
            ReadError::Refused(found) => {
                let lines: Vec<String> = found.iter().map(ToString::to_string).collect();
                f.write_str(&lines.join("; "))
            }
        }
    }
}

/// How an artifact type is identified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identity {
    /// The SHA-256 of the file's bytes.
    FileBytes,
    /// The image manifest digest of the layout.
    OciManifestDigest,
}

/// The identity of an artifact type, or `None` when the type is not accepted: a type is
/// accepted only once it names what its digest is of. The match has no wildcard, so a new type
/// does not compile until it is placed here.
pub fn identity(kind: ArtifactType) -> Option<Identity> {
    match kind {
        ArtifactType::Unspecified | ArtifactType::Conda | ArtifactType::Python => None,
        ArtifactType::File | ArtifactType::Npm => Some(Identity::FileBytes),
        ArtifactType::Oci => Some(Identity::OciManifestDigest),
    }
}

/// A machine that has passed every rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    machine: Machine,
}

impl Checked {
    /// Checks `machine` against every rule.
    pub fn read(machine: Machine) -> Result<Self, ReadError> {
        if machine.schema_version > SCHEMA_VERSION {
            return Err(ReadError::Version {
                found: machine.schema_version,
            });
        }
        let mut found = Findings(Vec::new());
        check_versions_and_names(&machine, &mut found);
        check_enums(&machine, &mut found);
        check_duplicates(&machine, &mut found);
        check_stages(&machine, &mut found);
        check_artifacts(&machine, &mut found);
        check_channels(&machine, &mut found);
        check_steps(&machine, &mut found);
        check_trust(&machine, &mut found);
        if found.0.is_empty() {
            Ok(Self { machine })
        } else {
            Err(ReadError::Refused(found.0))
        }
    }

    pub fn machine(&self) -> &Machine {
        &self.machine
    }

    pub fn stage(&self, name: &str) -> Option<&Stage> {
        stage_named(&self.machine, name)
    }

    pub fn artifact(&self, name: &str) -> Option<&ArtifactDecl> {
        self.machine.artifacts.iter().find(|a| a.name == name)
    }

    pub fn channel(&self, name: &str) -> Option<&Channel> {
        channel_named(&self.machine, name)
    }
}

/// The stage named `name`.
pub fn stage_named<'a>(machine: &'a Machine, name: &str) -> Option<&'a Stage> {
    machine.stages.iter().find(|s| s.name == name)
}

fn channel_named<'a>(machine: &'a Machine, name: &str) -> Option<&'a Channel> {
    machine.channels.iter().find(|c| c.name == name)
}

/// The platform a step names.
pub fn step_platform(step: &Step) -> Option<&str> {
    match &step.action {
        Some(Action::Build(b)) => Some(&b.platform),
        Some(Action::Publish(p)) => Some(&p.platform),
        None => None,
    }
}

/// The platform of a stage: the one its steps name.
pub fn stage_platform(stage: &Stage) -> Option<&str> {
    stage.steps.iter().find_map(step_platform)
}

/// The environment of a stage: its own, or its name when it sets none.
pub fn stage_environment(stage: &Stage) -> &str {
    if stage.environment.is_empty() {
        &stage.name
    } else {
        &stage.environment
    }
}

/// The BUILD steps of a stage with their actions, in order.
pub fn builds(stage: &Stage) -> impl Iterator<Item = (&Step, &Build)> {
    stage.steps.iter().filter_map(|s| match &s.action {
        Some(Action::Build(b)) => Some((s, b)),
        _ => None,
    })
}

/// The PUBLISH steps of a stage with their actions, in order.
pub fn publishes(stage: &Stage) -> impl Iterator<Item = (&Step, &Publish)> {
    stage.steps.iter().filter_map(|s| match &s.action {
        Some(Action::Publish(p)) => Some((s, p)),
        _ => None,
    })
}

/// The puts per coordinate a publish step may use: its own bound, or the default when it is 0.
pub fn put_attempts(publish: &Publish) -> u32 {
    if publish.attempts == 0 {
        DEFAULT_PUT_ATTEMPTS
    } else {
        publish.attempts
    }
}

/// The read-back polls per coordinate a publish step may use: its own bound, or the default.
pub fn readback_polls(publish: &Publish) -> u32 {
    if publish.polls == 0 {
        DEFAULT_READBACK_POLLS
    } else {
        publish.polls
    }
}

struct Findings(Vec<Refusal>);

impl Findings {
    fn add(&mut self, rule: Rule, message: String) {
        self.0.push(Refusal { rule, message });
    }
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
        for step in &stage.steps {
            name("the step", &step.name, false);
            step.validations
                .iter()
                .for_each(|v| name("the validation", &v.name, false));
        }
    }
    for stage in &m.stages {
        for platform in stage.steps.iter().filter_map(step_platform) {
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
        for step in &stage.steps {
            for v in &step.validations {
                if !matches!(
                    Subject::try_from(v.subject),
                    Ok(Subject::LocalSet | Subject::ChannelReadback)
                ) {
                    f.add(
                        Rule::UnknownEnum,
                        format!(
                            "validation {} has subject {}, which is not a subject",
                            v.name, v.subject
                        ),
                    );
                }
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
    for repo in m.channels.iter().flat_map(|c| &c.repositories) {
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

/// The names that occur more than once, each once, in order.
fn repeated<'a>(names: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut seen = BTreeSet::new();
    let mut twice = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            twice.insert(name);
        }
    }
    twice.into_iter().collect()
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
            "step",
            repeated(stage.steps.iter().map(|s| s.name.as_str())),
        );
        let validations = stage.steps.iter().flat_map(|s| &s.validations);
        report("validation", repeated(validations.map(|v| v.name.as_str())));
    }
}

fn check_stages(m: &Machine, f: &mut Findings) {
    let position: BTreeMap<&str, usize> = m
        .stages
        .iter()
        .enumerate()
        .map(|(at, s)| (s.name.as_str(), at))
        .collect();
    let needed: BTreeSet<&str> = m
        .stages
        .iter()
        .flat_map(|s| s.needs.iter().map(String::as_str))
        .collect();
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
        let platforms: BTreeSet<&str> = stage.steps.iter().filter_map(step_platform).collect();
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
                    "stage {} holds more than one BUILD step, and a stage seals one set",
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
                reject("holds a PUBLISH step");
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

fn check_artifacts(m: &Machine, f: &mut Findings) {
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

fn check_channels(m: &Machine, f: &mut Findings) {
    let mut locations: BTreeMap<&str, usize> = BTreeMap::new();
    for channel in &m.channels {
        if channel.repositories.is_empty() {
            f.add(
                Rule::ChannelShape,
                format!("channel {} has no repository", channel.name),
            );
        }
        let kinds = repeated(channel.repositories.iter().map(|r| type_text(r.r#type)));
        for kind in kinds {
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
    ArtifactType::try_from(raw).map_or("UNKNOWN", |t| t.as_str_name())
}

fn check_push(channel: &Channel, repo: &Repository, f: &mut Findings) {
    let fed = match repo.push.as_ref().and_then(|p| p.kind.as_ref()) {
        None => {
            f.add(
                Rule::ChannelShape,
                format!(
                    "the repository {} of channel {} has no push identity",
                    repo.location, channel.name
                ),
            );
            return;
        }
        Some(Kind::Anonymous(_)) => return,
        Some(Kind::Federated(fed)) => fed,
    };
    let mut refuse = |why: &str| {
        f.add(
            Rule::ExceptionalSubject,
            format!(
                "the repository {} of channel {} {why}",
                repo.location, channel.name
            ),
        );
    };
    if !fed.exceptional_subject.is_empty() && fed.exceptional_subject == fed.subject {
        refuse("has an exceptional subject equal to its subject");
    }
    if !fed.exceptional_environment.is_empty() && fed.exceptional_environment == fed.environment {
        refuse("has an exceptional environment equal to its environment");
    }
    if fed.exceptional_subject.is_empty() != fed.exceptional_environment.is_empty() {
        refuse("sets only one of its exceptional subject and exceptional environment");
    }
}

fn check_steps(m: &Machine, f: &mut Findings) {
    for stage in &m.stages {
        for (at, step) in stage.steps.iter().enumerate() {
            match &step.action {
                None => f.add(
                    Rule::StepWithoutAction,
                    format!(
                        "step {} of stage {} is neither a build nor a publish",
                        step.name, stage.name
                    ),
                ),
                Some(Action::Build(build)) => {
                    for name in build
                        .artifacts
                        .iter()
                        .filter(|n| m.artifacts.iter().all(|a| &a.name != *n))
                    {
                        f.add(
                            Rule::BuildUndeclared,
                            format!("step {} builds {name}, which is not declared", step.name),
                        );
                    }
                }
                Some(Action::Publish(publish)) => check_publish(m, step, publish, f),
            }
            for v in &step.validations {
                check_validation(stage, at, step, v, f);
            }
        }
    }
}

fn check_publish(m: &Machine, step: &Step, publish: &Publish, f: &mut Findings) {
    for (what, value) in [("attempts", publish.attempts), ("polls", publish.polls)] {
        if value > MAX_BOUND {
            f.add(
                Rule::BoundOutOfRange,
                format!("step {} has {what} {value}, above {MAX_BOUND}", step.name),
            );
        }
    }
    let channel = channel_named(m, &publish.channel);
    if channel.is_none() {
        let declared: Vec<&str> = m.channels.iter().map(|c| c.name.as_str()).collect();
        f.add(
            Rule::UnknownChannel,
            format!(
                "step {} publishes to {:?}, and the declared channels are [{}]",
                step.name,
                publish.channel,
                declared.join(", ")
            ),
        );
    }
    for name in &publish.artifacts {
        match m.artifacts.iter().find(|a| &a.name == name) {
            None => f.add(
                Rule::PublishUndeclared,
                format!("step {} publishes {name}, which is not declared", step.name),
            ),
            Some(artifact) => {
                if channel
                    .is_some_and(|c| c.repositories.iter().all(|r| r.r#type != artifact.r#type))
                {
                    f.add(
                        Rule::PublishUndeclared,
                        format!(
                            "channel {} has no repository for the type of {name}",
                            publish.channel
                        ),
                    );
                }
            }
        }
    }
}

fn check_validation(stage: &Stage, at: usize, step: &Step, v: &Validation, f: &mut Findings) {
    if v.wait_polls > MAX_BOUND {
        f.add(
            Rule::BoundOutOfRange,
            format!(
                "validation {} has wait_polls {}, above {MAX_BOUND}",
                v.name, v.wait_polls
            ),
        );
    }
    let is_build = matches!(step.action, Some(Action::Build(_)));
    if v.subject == Subject::ChannelReadback as i32 && is_build {
        f.add(
            Rule::ReadbackOnBuild,
            format!(
                "validation {} reads back a channel on the BUILD step {}",
                v.name, step.name
            ),
        );
    }
    if v.subject == Subject::LocalSet as i32
        && !stage.steps[..=at]
            .iter()
            .any(|s| matches!(s.action, Some(Action::Build(_))))
    {
        f.add(
            Rule::LocalSetWithoutBuild,
            format!(
                "validation {} reads a local set, and no BUILD step of stage {} comes before it",
                v.name, stage.name
            ),
        );
    }
}

/// The stages `stage` needs that hold a BUILD step.
pub fn needed_builds<'a>(machine: &'a Machine, stage: &Stage) -> Vec<&'a Stage> {
    stage
        .needs
        .iter()
        .filter_map(|n| stage_named(machine, n))
        .filter(|s| builds(s).next().is_some())
        .collect()
}

fn check_trust(m: &Machine, f: &mut Findings) {
    let needed: BTreeSet<&str> = m
        .stages
        .iter()
        .flat_map(|s| s.needs.iter().map(String::as_str))
        .collect();
    for stage in m.stages.iter().filter(|s| publishes(s).next().is_some()) {
        let name = &stage.name;
        if builds(stage).next().is_some() {
            f.add(
                Rule::PublishStageHasBuild,
                format!("stage {name} holds both a PUBLISH step and a BUILD step"),
            );
        }
        let have = needed_builds(m, stage);
        match have.as_slice() {
            [build_stage] => check_against_build(stage, build_stage, f),
            _ => f.add(
                Rule::PublishStageNeedsValidatedBuild,
                format!("stage {name} needs {} stages that hold a BUILD step, and it must need exactly one", have.len()),
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
            && !stage
                .steps
                .iter()
                .flat_map(|s| &s.validations)
                .any(|v| v.subject == Subject::ChannelReadback as i32)
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
    let closed = builds(build_stage).any(|(step, _)| {
        step.validations
            .iter()
            .any(|v| v.subject == Subject::LocalSet as i32 && v.validator == SET_CLOSURE)
    });
    if !closed {
        f.add(
            Rule::PublishStageNeedsValidatedBuild,
            format!("stage {} builds the set of stage {}, which has no {SET_CLOSURE} validation of its local set", stage.name, build_stage.name),
        );
    }
    let declared: BTreeSet<&str> = builds(build_stage)
        .flat_map(|(_, b)| b.artifacts.iter().map(String::as_str))
        .collect();
    let platform = stage_platform(build_stage);
    for (step, publish) in publishes(stage) {
        if Some(publish.platform.as_str()) != platform {
            f.add(
                Rule::PublishStageScope,
                format!(
                    "step {} publishes for {:?}, and stage {} builds for {:?}",
                    step.name,
                    publish.platform,
                    build_stage.name,
                    platform.unwrap_or("")
                ),
            );
        }
        for name in publish
            .artifacts
            .iter()
            .filter(|n| !declared.contains(n.as_str()))
        {
            f.add(
                Rule::PublishStageScope,
                format!(
                    "step {} publishes {name}, which stage {} does not build",
                    step.name, build_stage.name
                ),
            );
        }
    }
}

fn check_environment(m: &Machine, stage: &Stage, f: &mut Findings) {
    let env = stage_environment(stage);
    for (step, publish) in publishes(stage) {
        let Some(channel) = channel_named(m, &publish.channel) else {
            continue;
        };
        let kinds: BTreeSet<i32> = publish
            .artifacts
            .iter()
            .filter_map(|n| m.artifacts.iter().find(|a| &a.name == n))
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
                    format!("step {} publishes to {}, whose identity names environment {:?}, and stage {} runs in {env:?}", step.name, repo.location, fed.environment, stage.name),
                );
            }
        }
    }
}
