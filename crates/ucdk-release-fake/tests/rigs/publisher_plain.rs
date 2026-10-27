// SPDX-License-Identifier: Apache-2.0
//! A publisher rig that takes the kit's default of every hook.

use ucdk_release::codec::GrantPhase;
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::Grant;
use ucdk_release_seams::kit::{Arrangement, ChannelShape, PublisherRig, ReadStep};

use super::publisher::{FakePublisherRig, Flawed};

/// A rig that writes what the kit requires of one and nothing else: every hook is the kit's own
/// default, so the coordinates and bytes are the literal ones, the siblings are every field in a
/// set of its own, no coordinate is unrepresentable, no version is sealed and no number burns.
pub struct PlainRig(FakePublisherRig);

impl PlainRig {
    pub fn new() -> Self {
        Self(FakePublisherRig::conforming())
    }
}

impl PublisherRig for PlainRig {
    type Publisher = Flawed;

    fn shapes(&self) -> Vec<ChannelShape> {
        self.0.shapes()
    }

    fn publisher(&self, shape: &ChannelShape) -> Flawed {
        self.0.publisher(shape)
    }

    fn arrange(&self, p: &mut Flawed, what: &Arrangement) {
        self.0.arrange(p, what);
    }

    fn grant(&self, p: &Flawed, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        self.0.grant(p, phase, coordinates)
    }

    fn foreign_grant(&self, p: &Flawed, phase: GrantPhase, coordinates: &[Coordinate]) -> Grant {
        self.0.foreign_grant(p, phase, coordinates)
    }

    fn advance(&self, ms: u64) {
        self.0.advance(ms);
    }

    fn reads(&self, p: &Flawed) -> Vec<ReadStep> {
        self.0.reads(p)
    }

    fn sent(&self, p: &Flawed) -> usize {
        self.0.sent(p)
    }
}
