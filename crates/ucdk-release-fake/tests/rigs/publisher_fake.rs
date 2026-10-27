// SPDX-License-Identifier: Apache-2.0
//! The publisher rig: the fake registry in each shape, the grants its issuer gives, and the
//! declarations a rig makes about itself. The stubs of the registry are in `publisher`.

use std::time::Duration;

use ucdk_core::ManualClock;
use ucdk_release::codec::GrantPhase;
use ucdk_release_fake::{FakeRegistry, Read};
use ucdk_release_proto::v1::{ArtifactType, Coordinate, Repository};
use ucdk_release_seams::kit::{
    Arrangement, ChannelShape, LOCATION, Payload, Placement, PublisherRig, ReadStep, Sibling,
    coordinate, every_field_in_a_set_of_its_own, federated_repository,
};
use ucdk_release_seams::{Grant, Publisher};

use super::publisher::Flawed;
use super::publisher_flaw::{Behind, Flaw, Hook};
use super::publisher_grant::issue;

/// The platform of the coordinate the conforming rig's channel cannot hold.
const FOREIGN_PLATFORM: &str = "plan9-mips";

/// A rig of the fake registry. It declares the shapes it builds and says of its grammar what is
/// true of it; a builder changes a declaration, or the registry behind it, one way at a time.
pub struct FakePublisherRig {
    clock: ManualClock,
    flaw: Option<Flaw>,
    /// The stub is behind the channels of this kind only, when the rig says.
    behind: Option<Behind>,
    hook: Option<Hook>,
    shapes: Vec<ChannelShape>,
    /// The rig says a version is sealed by its first group.
    declares_seals: bool,
    /// The registry seals a version by its first group.
    seals: bool,
    per_coordinate: bool,
    unconditional: bool,
    version_yank: bool,
    narrow: bool,
    burns: bool,
    /// The channel holds every coordinate: nothing is unrepresentable.
    holds_all: bool,
}

impl FakePublisherRig {
    /// The fake in every shape the kits use, with literal coordinates and bytes.
    pub fn conforming() -> Self {
        Self {
            clock: ManualClock::at(1_000_000),
            flaw: None,
            behind: None,
            hook: None,
            shapes: vec![
                ChannelShape::staging(),
                ChannelShape::without_staging(),
                ChannelShape::public_grouped(),
                ChannelShape::private_grouped(),
                ChannelShape {
                    group_addressed: false,
                    ..ChannelShape::private_grouped()
                },
                ChannelShape {
                    stages: false,
                    group_addressed: false,
                    ..ChannelShape::private_grouped()
                },
            ],
            declares_seals: false,
            seals: false,
            per_coordinate: false,
            unconditional: false,
            version_yank: false,
            narrow: false,
            burns: true,
            holds_all: false,
        }
    }

    /// A channel with no grammar to refuse by: it holds every coordinate.
    pub fn holding_every_coordinate() -> Self {
        Self {
            holds_all: true,
            ..Self::conforming()
        }
    }

    pub fn flawed(flaw: Flaw) -> Self {
        Self::conforming().with_flaw(flaw)
    }

    /// The same rig with the stub of `flaw` behind it.
    #[must_use]
    pub fn with_flaw(self, flaw: Flaw) -> Self {
        let version_yank = self.version_yank
            || matches!(
                flaw,
                Flaw::VersionYankIsPerCoordinate
                    | Flaw::VersionYankIgnoresTheGrant
                    | Flaw::VersionYankReachesEveryVersion
            );
        Self {
            flaw: Some(flaw),
            version_yank,
            ..self
        }
    }

    /// The same rig with the stub of `flaw` behind the channels of one kind only: the channels of
    /// the other shapes are the conforming fake.
    #[must_use]
    pub fn with_flaw_behind(self, flaw: Flaw, behind: Behind) -> Self {
        Self {
            behind: Some(behind),
            ..self.with_flaw(flaw)
        }
    }

    /// The same rig making a wrong declaration of its coordinates, bytes or siblings.
    #[must_use]
    pub fn hooked(self, hook: Hook) -> Self {
        Self {
            hook: Some(hook),
            ..self
        }
    }

    /// The same rig declaring only `shapes`.
    #[must_use]
    pub fn with_shapes(self, shapes: Vec<ChannelShape>) -> Self {
        Self { shapes, ..self }
    }

    /// A rig that builds one public channel that does not stage, one call to a commit.
    pub fn restricted() -> Self {
        Self::conforming().with_shapes(vec![ChannelShape::without_staging()])
    }

    /// A registry that seals a version by its first group, and a rig that says so.
    pub fn sealing() -> Self {
        Self {
            declares_seals: true,
            seals: true,
            ..Self::conforming()
        }
    }

    /// A rig that says the registry seals a version, which it does not.
    pub fn declaring_seals_only() -> Self {
        Self {
            declares_seals: true,
            ..Self::conforming()
        }
    }

    /// A registry that seals a version, which the rig does not say.
    pub fn sealing_undeclared() -> Self {
        Self {
            seals: true,
            ..Self::conforming()
        }
    }

    /// A registry whose commit may replace what is there, and a rig that says so.
    pub fn unconditional() -> Self {
        Self {
            unconditional: true,
            ..Self::conforming()
        }
    }

    /// A registry that commits each coordinate on its own.
    pub fn per_coordinate() -> Self {
        Self {
            per_coordinate: true,
            ..Self::conforming()
        }
    }

    /// A registry that yanks every platform of a version at one call.
    pub fn yanking_a_version() -> Self {
        Self {
            version_yank: true,
            ..Self::conforming()
        }
    }

    /// A registry with no failed groups: it cannot burn a number.
    pub fn unable_to_burn() -> Self {
        Self {
            burns: false,
            ..Self::conforming()
        }
    }

    /// A registry with a grammar of its own, as a real one has: names of two parts, one
    /// platform spelling, a member name that is a file name, one platform that is published with
    /// the base, and a version that is sealed by the group that first wrote it.
    pub fn narrow() -> Self {
        Self {
            narrow: true,
            declares_seals: true,
            seals: true,
            ..Self::conforming()
        }
    }

    /// A registry with a grammar of its own that yanks every platform of a version at one call,
    /// as a registry that seals a version by its first group does.
    pub fn narrow_yanking_a_version() -> Self {
        Self {
            version_yank: true,
            ..Self::narrow()
        }
    }

    pub(super) fn issue(
        &self,
        repository: &Repository,
        phase: GrantPhase,
        coordinates: &[Coordinate],
    ) -> Grant {
        issue(&self.clock, repository, phase, coordinates)
    }

    /// The coordinate the registry cannot hold, as it is really.
    fn banned(&self) -> Vec<Coordinate> {
        if self.holds_all {
            return Vec::new();
        }
        vec![Coordinate {
            platform: if self.narrow {
                "win-32".to_owned()
            } else {
                FOREIGN_PLATFORM.to_owned()
            },
            ..self.coordinate("a")
        }]
    }

    fn is(&self, hook: Hook) -> bool {
        self.hook == Some(hook)
    }
}

impl PublisherRig for FakePublisherRig {
    type Publisher = Flawed;

    fn shapes(&self) -> Vec<ChannelShape> {
        self.shapes.clone()
    }

    fn publisher(&self, shape: &ChannelShape) -> Flawed {
        let flaw = self
            .flaw
            .filter(|_| self.behind.is_none_or(|behind| behind.holds(shape)));
        let mut registry = FakeRegistry::new(&self.clock, LOCATION);
        if !shape.stages {
            registry = registry.without_staging();
        }
        if shape.private {
            registry = registry.private();
        }
        if shape.group_addressed {
            registry = registry.group_addressed();
        }
        if self.per_coordinate || flaw == Some(Flaw::CommitUnitSaysSet) {
            registry = registry.commit_per_coordinate();
        }
        if self.unconditional {
            registry = registry.tag_replacing();
        }
        Flawed::new(
            registry,
            self.clock.clone(),
            self.banned(),
            flaw,
            *shape,
            (self.seals, self.version_yank),
        )
    }

    fn arrange(&self, p: &mut Flawed, what: &Arrangement) {
        let next_commit = p.commit_calls + 1;
        match what {
            Arrangement::HeldByAnother { coordinate, bytes } => {
                p.inner.present_different(coordinate, bytes);
            }
            Arrangement::StagedByAnother { coordinate, bytes } => {
                p.inner.stage_other_bytes(coordinate, bytes);
            }
            Arrangement::RaceNextCommit(bytes) => {
                p.raced = true;
                p.inner.race_next_commit(bytes);
            }
            Arrangement::LoseAnswerAfterCommit => p.inner.lose_answer_after_commit(next_commit),
            Arrangement::LoseAnswerBeforeCommit => p.inner.lose_answer_before_commit(next_commit),
            Arrangement::FailAfterCommit => p.inner.fail_after_commit(next_commit),
            Arrangement::CommitSetFailsWhole => {
                if p.flaw == Some(Flaw::SetCommitIsTorn) && p.shape.stages {
                    p.inner.torn_commit();
                } else {
                    p.inner.commit_set_fails_whole();
                }
            }
            Arrangement::ListLags => {
                p.list_lags = true;
                p.inner.list_lag();
            }
            Arrangement::FailGroupRead => {
                p.group_fails = true;
                p.inner.fail_group_read();
            }
            Arrangement::FailIndexRead => p.inner.fail_index_read(),
            Arrangement::LoseStagedState => {
                p.vanished = true;
                let staged = p.known_where(|me, c| me.inner.is_staged(c));
                let grant = issue(
                    &p.clock,
                    &federated_repository(),
                    GrantPhase::Stage,
                    &staged,
                );
                p.inner
                    .abort(&grant, &staged)
                    .expect("the registry gives back what it staged");
            }
            Arrangement::CommitVisibleAfter(n) => {
                p.becoming = true;
                p.inner.read_lag(*n);
            }
            Arrangement::BurnedByFailedGroup { coordinate } => {
                p.burned.push(coordinate.clone());
                p.inner.burn(coordinate);
            }
        }
    }

    fn grant(&self, _p: &Flawed, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        self.issue(&federated_repository(), phase, coordinates)
    }

    fn foreign_grant(&self, _p: &Flawed, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        let elsewhere = Repository {
            location: "registry/other".to_owned(),
            r#type: ArtifactType::File as i32,
            ..federated_repository()
        };
        self.issue(&elsewhere, phase, coordinates)
    }

    fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
    }

    fn reads(&self, p: &Flawed) -> Vec<ReadStep> {
        let steps: Vec<ReadStep> = p
            .inner
            .read_log()
            .iter()
            .map(|read| match read {
                Read::Group => ReadStep::Group,
                Read::Index => ReadStep::Index,
            })
            .collect();
        if p.flaw != Some(Flaw::IndexIsReadBeforeTheGroup) {
            return steps;
        }
        let mut swapped = Vec::new();
        let mut at = 0;
        while at < steps.len() {
            if steps[at] == ReadStep::Group && steps.get(at + 1) == Some(&ReadStep::Index) {
                swapped.extend([ReadStep::Index, ReadStep::Group]);
                at += 2;
            } else {
                swapped.push(steps[at]);
                at += 1;
            }
        }
        swapped
    }

    fn coordinate(&self, label: &str) -> Coordinate {
        if self.narrow {
            Coordinate {
                name: format!("{label}_pkg"),
                platform: "linux-64".to_owned(),
                ..coordinate(label)
            }
        } else if self.is(Hook::CoordinateDropsThePrefix) {
            coordinate(&format!("x{label}"))
        } else {
            coordinate(label)
        }
    }

    fn payload(&self, k: &Coordinate, seed: u8) -> Payload {
        let seed = if self.is(Hook::PayloadIgnoresTheSeed) {
            0
        } else {
            seed
        };
        let mut bytes = if self.is(Hook::PayloadIsEmpty) {
            Vec::new()
        } else {
            format!("{}-{}-{}", k.name, k.version, k.platform).into_bytes()
        };
        if seed != 0 && !bytes.is_empty() {
            bytes.push(seed);
        }
        let name = if self.narrow {
            format!("mojo-{}-{}-0.conda", k.name, k.version)
        } else {
            k.name.clone()
        };
        Payload { name, bytes }
    }

    fn siblings(&self, base: &Coordinate) -> Vec<Sibling> {
        let platform = Coordinate {
            platform: "osx-arm64".to_owned(),
            ..base.clone()
        };
        let version = Coordinate {
            version: "1.2.4".to_owned(),
            ..base.clone()
        };
        if self.narrow {
            return vec![
                Sibling {
                    field: "platform",
                    coordinate: platform,
                    placement: Placement::SameSet,
                },
                Sibling {
                    field: "version",
                    coordinate: version,
                    placement: Placement::OwnSet,
                },
            ];
        }
        let mut all = every_field_in_a_set_of_its_own(base);
        match self.hook {
            Some(Hook::SiblingDiffersInTwoFields) => {
                all[0].coordinate = Coordinate {
                    version: "1.2.4".to_owned(),
                    ..all[0].coordinate.clone()
                };
            }
            Some(Hook::SiblingIsTheBase) => all[0].coordinate = base.clone(),
            _ => {}
        }
        all
    }

    fn unrepresentable(&self) -> Vec<Coordinate> {
        if self.is(Hook::UnrepresentableIsTheBase) {
            vec![self.coordinate("a")]
        } else {
            self.banned()
        }
    }

    fn seals_versions(&self) -> bool {
        self.declares_seals
    }

    fn can_burn(&self) -> bool {
        self.burns
    }
}
