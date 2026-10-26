// SPDX-License-Identifier: Apache-2.0
//! Reading a machine: every rule a declared machine must satisfy, each refusal named by a
//! stable rule id (`machine.<name>`), so a test and a caller never parse a message.
//!
//! [`Checked::read`] is the only way to a [`Checked`] machine, so a value of that type has
//! passed every rule here. The rules are written as the condition that is refused. They are
//! grouped by what they read: names, enums and the stage graph (`graph`), the build system,
//! targets and the actions that run it (`building`), and channels, publishing and the places a
//! validation may sit (`publishing`).

mod building;
mod graph;
mod publishing;

use std::collections::BTreeSet;
use std::fmt;

use ucdk_release_proto::v1::action::Kind as ActionKind;
use ucdk_release_proto::v1::{
    Action, ArtifactDecl, ArtifactType, Build, Channel, Machine, Publish, Reason, Smoke, Stage,
    Test, Validation,
};

/// The schema version this reader understands.
pub const SCHEMA_VERSION: i32 = 1;
/// The calls per phase a publish action may use when it states none.
pub const DEFAULT_PUT_ATTEMPTS: u32 = 3;
/// The read-back polls per coordinate a publish action may use when it states none.
pub const DEFAULT_READBACK_POLLS: u32 = 3;
/// The greatest value of any authored bound.
pub const MAX_BOUND: u32 = 10;
/// The most targets one Test action may name.
pub const MAX_TEST_TARGETS: usize = 32;

/// The rules of reading a machine. Each is one refused condition and has one stable id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    SchemaVersion,
    BadName,
    UnknownEnum,
    UnknownStage,
    StageCycle,
    ActionWithoutKind,
    StageWithoutActions,
    EmptyArtifactList,
    DuplicateName,
    UnknownChannel,
    BuildSystemRequired,
    BuildSystemShape,
    ReservedFlag,
    EnvKey,
    Placeholder,
    TargetShape,
    ArtifactTargetMissing,
    BuildUndeclared,
    BuildNotClosed,
    LabelsOnNonFile,
    TestShape,
    ChangeCheckPublishes,
    StagePlatform,
    ArtifactCycle,
    ChannelShape,
    ExceptionalSubject,
    FederatedShape,
    PublishUndeclared,
    ValidationPlacement,
    SmokeShape,
    EnvironmentMismatch,
    PublishStageRunsCode,
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
    pub const ALL: [Rule; 39] = [
        Rule::SchemaVersion,
        Rule::BadName,
        Rule::UnknownEnum,
        Rule::UnknownStage,
        Rule::StageCycle,
        Rule::ActionWithoutKind,
        Rule::StageWithoutActions,
        Rule::EmptyArtifactList,
        Rule::DuplicateName,
        Rule::UnknownChannel,
        Rule::BuildSystemRequired,
        Rule::BuildSystemShape,
        Rule::ReservedFlag,
        Rule::EnvKey,
        Rule::Placeholder,
        Rule::TargetShape,
        Rule::ArtifactTargetMissing,
        Rule::BuildUndeclared,
        Rule::BuildNotClosed,
        Rule::LabelsOnNonFile,
        Rule::TestShape,
        Rule::ChangeCheckPublishes,
        Rule::StagePlatform,
        Rule::ArtifactCycle,
        Rule::ChannelShape,
        Rule::ExceptionalSubject,
        Rule::FederatedShape,
        Rule::PublishUndeclared,
        Rule::ValidationPlacement,
        Rule::SmokeShape,
        Rule::EnvironmentMismatch,
        Rule::PublishStageRunsCode,
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
            Rule::UnknownStage => "machine.unknown_stage",
            Rule::StageCycle => "machine.stage_cycle",
            Rule::ActionWithoutKind => "machine.action_without_kind",
            Rule::StageWithoutActions => "machine.stage_without_actions",
            Rule::EmptyArtifactList => "machine.empty_artifact_list",
            Rule::DuplicateName => "machine.duplicate_name",
            Rule::UnknownChannel => "machine.unknown_channel",
            Rule::BuildSystemRequired => "machine.build_system_required",
            Rule::BuildSystemShape => "machine.build_system_shape",
            Rule::ReservedFlag => "machine.reserved_flag",
            Rule::EnvKey => "machine.env_key",
            Rule::Placeholder => "machine.placeholder",
            Rule::TargetShape => "machine.target_shape",
            Rule::ArtifactTargetMissing => "machine.artifact_target_missing",
            Rule::BuildUndeclared => "machine.build_undeclared",
            Rule::BuildNotClosed => "machine.build_not_closed",
            Rule::LabelsOnNonFile => "machine.labels_on_non_file",
            Rule::TestShape => "machine.test_shape",
            Rule::ChangeCheckPublishes => "machine.change_check_publishes",
            Rule::StagePlatform => "machine.stage_platform",
            Rule::ArtifactCycle => "machine.artifact_cycle",
            Rule::ChannelShape => "machine.channel_shape",
            Rule::ExceptionalSubject => "machine.exceptional_subject",
            Rule::FederatedShape => "machine.federated_shape",
            Rule::PublishUndeclared => "machine.publish_undeclared",
            Rule::ValidationPlacement => "machine.validation_placement",
            Rule::SmokeShape => "machine.smoke_shape",
            Rule::EnvironmentMismatch => "machine.environment_mismatch",
            Rule::PublishStageRunsCode => "machine.publish_stage_runs_code",
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
        graph::check(&machine, &mut found);
        building::check(&machine, &mut found);
        publishing::check(&machine, &mut found);
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
        artifact_named(&self.machine, name)
    }

    pub fn channel(&self, name: &str) -> Option<&Channel> {
        channel_named(&self.machine, name)
    }
}

/// The stage named `name`.
pub fn stage_named<'a>(machine: &'a Machine, name: &str) -> Option<&'a Stage> {
    machine.stages.iter().find(|s| s.name == name)
}

/// The artifact declared as `name`.
pub fn artifact_named<'a>(machine: &'a Machine, name: &str) -> Option<&'a ArtifactDecl> {
    machine.artifacts.iter().find(|a| a.name == name)
}

/// The channel declared as `name`.
pub fn channel_named<'a>(machine: &'a Machine, name: &str) -> Option<&'a Channel> {
    machine.channels.iter().find(|c| c.name == name)
}

/// The platform an action names; a Test action names none.
pub fn action_platform(action: &Action) -> Option<&str> {
    match &action.kind {
        Some(ActionKind::Build(b)) => Some(&b.platform),
        Some(ActionKind::Publish(p)) => Some(&p.platform),
        Some(ActionKind::Smoke(s)) => Some(&s.platform),
        Some(ActionKind::Test(_)) | None => None,
    }
}

/// The platform of a stage: the one its actions name.
pub fn stage_platform(stage: &Stage) -> Option<&str> {
    stage.actions.iter().find_map(action_platform)
}

/// The environment of a stage: its own, or its name when it sets none.
pub fn stage_environment(stage: &Stage) -> &str {
    if stage.environment.is_empty() {
        &stage.name
    } else {
        &stage.environment
    }
}

/// The Build actions of a stage with their bodies, in order.
pub fn builds(stage: &Stage) -> impl Iterator<Item = (&Action, &Build)> {
    stage.actions.iter().filter_map(|a| match &a.kind {
        Some(ActionKind::Build(b)) => Some((a, b)),
        _ => None,
    })
}

/// The Test actions of a stage with their bodies, in order.
pub fn tests(stage: &Stage) -> impl Iterator<Item = (&Action, &Test)> {
    stage.actions.iter().filter_map(|a| match &a.kind {
        Some(ActionKind::Test(t)) => Some((a, t)),
        _ => None,
    })
}

/// The Publish actions of a stage with their bodies, in order.
pub fn publishes(stage: &Stage) -> impl Iterator<Item = (&Action, &Publish)> {
    stage.actions.iter().filter_map(|a| match &a.kind {
        Some(ActionKind::Publish(p)) => Some((a, p)),
        _ => None,
    })
}

/// The Smoke actions of a stage with their bodies, in order.
pub fn smokes(stage: &Stage) -> impl Iterator<Item = (&Action, &Smoke)> {
    stage.actions.iter().filter_map(|a| match &a.kind {
        Some(ActionKind::Smoke(s)) => Some((a, s)),
        _ => None,
    })
}

/// Every validation of a stage: those of its actions in order, then the stage's own.
pub fn validations(stage: &Stage) -> impl Iterator<Item = &Validation> {
    stage
        .actions
        .iter()
        .flat_map(|a| &a.validations)
        .chain(&stage.validations)
}

/// The calls per phase a publish action may use: its own bound, or the default when it is 0.
pub fn put_attempts(publish: &Publish) -> u32 {
    if publish.attempts == 0 {
        DEFAULT_PUT_ATTEMPTS
    } else {
        publish.attempts
    }
}

/// The read-back polls per coordinate a publish action may use: its own bound, or the default.
pub fn readback_polls(publish: &Publish) -> u32 {
    if publish.polls == 0 {
        DEFAULT_READBACK_POLLS
    } else {
        publish.polls
    }
}

/// The stages `stage` needs that hold a Build action.
pub fn needed_builds<'a>(machine: &'a Machine, stage: &Stage) -> Vec<&'a Stage> {
    stage
        .needs
        .iter()
        .filter_map(|n| stage_named(machine, n))
        .filter(|s| builds(s).next().is_some())
        .collect()
}

/// The refusals found so far.
struct Findings(Vec<Refusal>);

impl Findings {
    fn add(&mut self, rule: Rule, message: String) {
        self.0.push(Refusal { rule, message });
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

/// The stages that another stage needs.
fn needed_names(m: &Machine) -> BTreeSet<&str> {
    m.stages
        .iter()
        .flat_map(|s| s.needs.iter().map(String::as_str))
        .collect()
}
