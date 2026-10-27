// SPDX-License-Identifier: Apache-2.0
//! The stubs of a registry that is lax about a grant on one verb, in one field of a coordinate or
//! on one shape of channel: it takes a grant the conforming fake refuses for one it accepts, and
//! is the conforming fake otherwise.

use ucdk_core::Clock;
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_fake::coordinate_key;
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::kit::LOCATION;
use ucdk_release_seams::{Grant, SeamError};

use super::publisher_more::{Field, More, Wrapped, differs_only_in};
use super::secret::texts;

/// A call of a registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Stage,
    Commit,
    Abort,
    Yank,
    Observe,
    Fetch,
    Newest,
    List,
}

impl Verb {
    /// Every verb.
    pub const ALL: [Self; 8] = [
        Self::Stage,
        Self::Commit,
        Self::Abort,
        Self::Yank,
        Self::Observe,
        Self::Fetch,
        Self::Newest,
        Self::List,
    ];

    /// The verbs whose call names a coordinate, which a grant may name or leave out.
    pub const NAMING: [Self; 6] = [
        Self::Stage,
        Self::Commit,
        Self::Abort,
        Self::Yank,
        Self::Observe,
        Self::Fetch,
    ];

    fn writes(self) -> bool {
        matches!(self, Self::Stage | Self::Commit | Self::Abort | Self::Yank)
    }
}

/// How a grant is bad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defect {
    /// It is for another repository.
    Foreign,
    /// It has expired.
    Expired,
    /// It is of the right phase and does not name the coordinates of the call.
    Uncovered,
    /// It is the grant of the other write phase (a read grant is no write grant of any phase).
    OtherWritePhase,
}

/// The shape of channel a stub is lax on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A private channel that stages and addresses staged state by coordinate.
    Private,
    /// A channel that addresses staged state by group.
    Grouped,
    /// A channel that does not stage.
    NonStaging,
}

/// Every stub that takes a grant naming a coordinate that differs in one field.
pub fn field_blind() -> Vec<More> {
    let mut all = Vec::new();
    for field in Field::ALL {
        for verb in Verb::NAMING {
            all.push(More::GrantBlindToField(field, verb));
        }
    }
    all
}

/// Every stub that is lax about a bad grant on a shape of channel other than the public one that
/// stages.
pub fn on_shapes() -> Vec<More> {
    let mut all = Vec::new();
    for kind in [Kind::Private, Kind::Grouped, Kind::NonStaging] {
        for defect in [
            Defect::Foreign,
            Defect::Expired,
            Defect::Uncovered,
            Defect::OtherWritePhase,
        ] {
            for verb in Verb::ALL {
                // A read has no phase to get wrong, and `newest` and `list` name no coordinate
                // for a grant to leave out; a channel that does not stage is never asked to
                // stage or abort; the reads of a private plain channel are held by the stubs
                // that take a read grant for another.
                let names_nothing = matches!(verb, Verb::Newest | Verb::List);
                let kept = match kind {
                    Kind::Private => verb.writes(),
                    Kind::Grouped => {
                        verb.writes()
                            || (defect != Defect::OtherWritePhase
                                && !(names_nothing && defect == Defect::Uncovered))
                    }
                    Kind::NonStaging => {
                        matches!(verb, Verb::Commit | Verb::Yank)
                            || (!verb.writes()
                                && defect != Defect::OtherWritePhase
                                && !(names_nothing && defect == Defect::Uncovered))
                    }
                };
                if kept {
                    all.push(More::BadGrantTakenOnShape(kind, defect, verb));
                }
            }
        }
    }
    all
}

/// The coordinate a text of a grant names.
fn parse_key(key: &str) -> Option<Coordinate> {
    let [channel, kind, name, version, platform] = texts(key.as_bytes()).try_into().ok()?;
    Some(Coordinate {
        channel,
        r#type: kind.parse().ok()?,
        name,
        version,
        platform,
    })
}

impl Wrapped {
    fn on(&self, kind: Kind) -> bool {
        match kind {
            Kind::Private => self.shape.private && !self.shape.group_addressed && self.shape.stages,
            Kind::Grouped => self.shape.group_addressed,
            Kind::NonStaging => !self.shape.stages,
        }
    }

    /// In what way `g` is bad for `verb` over `coordinates`.
    fn defect(&self, g: &Grant, verb: Verb, coordinates: &[Coordinate]) -> Option<Defect> {
        let text = texts(g.secret());
        let scope = text.first().map_or("", String::as_str);
        let repository = text.get(1).map_or("", String::as_str);
        let needed = match verb {
            Verb::Stage | Verb::Abort => Some("stage"),
            Verb::Commit | Verb::Yank => Some("commit"),
            Verb::Observe | Verb::Fetch | Verb::Newest | Verb::List => None,
        };
        if g.expires_ms()
            .is_some_and(|e| self.base.clock.now_millis() >= e)
        {
            return Some(Defect::Expired);
        }
        if repository != LOCATION {
            return Some(Defect::Foreign);
        }
        if let Some(needed) = needed
            && scope != needed
            && scope != "read"
        {
            return Some(Defect::OtherWritePhase);
        }
        let named = text.get(2..).unwrap_or_default();
        coordinates
            .iter()
            .any(|c| !named.contains(&coordinate_key(c)))
            .then_some(Defect::Uncovered)
    }

    /// Whether `g` names, for every coordinate of the call, one that differs from it in `field`
    /// only.
    fn names_a_sibling(g: &Grant, coordinates: &[Coordinate], field: Field) -> bool {
        let named: Vec<Coordinate> = texts(g.secret())
            .iter()
            .skip(2)
            .filter_map(|text| parse_key(text))
            .collect();
        coordinates
            .iter()
            .all(|c| named.iter().any(|n| differs_only_in(n, c, field)))
    }

    /// The grant a lax stub takes `g` for, when it takes it for another: the refusal of `result`
    /// is one it lifts.
    pub(super) fn lenient<T>(
        &self,
        verb: Verb,
        g: &Grant,
        coordinates: &[Coordinate],
        result: &Result<T, SeamError>,
    ) -> Option<Grant> {
        let Err(SeamError::GrantRefused(GrantFault::Denied | GrantFault::Expired)) = result else {
            return None;
        };
        if g.is_none() {
            return None;
        }
        let take = match self.flaw() {
            Some(More::GrantBlindToField(field, v)) => {
                v == verb && Self::names_a_sibling(g, coordinates, field)
            }
            Some(More::BadGrantTakenOnShape(kind, defect, v)) => {
                v == verb && self.on(kind) && self.defect(g, verb, coordinates) == Some(defect)
            }
            _ => false,
        };
        let phase = match verb {
            Verb::Stage | Verb::Abort => GrantPhase::Stage,
            Verb::Commit | Verb::Yank => GrantPhase::Commit,
            Verb::Observe | Verb::Fetch | Verb::Newest | Verb::List => GrantPhase::Read,
        };
        take.then(|| self.internal(phase, coordinates))
    }
}
