// SPDX-License-Identifier: Apache-2.0
//! The fake registry's burned coordinate: what a registry answers for a number a failed group
//! claimed, and that it answers `Burned` and never `Absent`; and the unit of its yank.

mod common;

use common::*;
use ucdk_core::ManualClock;
use ucdk_release::codec::GrantPhase;
use ucdk_release::publish::{Presence, PublishDecision, decide_publish};
use ucdk_release_fake::FakeRegistry;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{
    CommitAnswer, Grant, ListScope, Publisher, SeamError, StageAnswer, YankUnit,
};

const OURS: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[test]
fn a_burned_coordinate_is_observed_as_burned_by_a_stranger_and_by_a_grant() {
    let mut w = World::new();
    let set = Set::of(&["a", "b"]);
    w.registry.burn(&set.coords[0]);
    // A stranger on a public registry, a read grant and a stage grant all see it burned.
    assert_eq!(
        w.registry.observe(&Grant::none(), &set.coords[0], &[]),
        Ok(Presence::Burned)
    );
    for phase in [GrantPhase::Read, GrantPhase::Stage, GrantPhase::Commit] {
        let g = w.grant(phase, &set.coords);
        assert_eq!(
            w.registry.observe(&g, &set.coords[0], &[]),
            Ok(Presence::Burned),
            "{phase:?}"
        );
    }
    // The declared set changes nothing, and a burn is of the one coordinate: its sibling is
    // still absent.
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    let items = set.items(&blobs);
    assert_eq!(
        w.registry.observe(&g, &set.coords[0], &items),
        Ok(Presence::Burned)
    );
    assert_eq!(
        w.registry.observe(&g, &set.coords[1], &items),
        Ok(Presence::Absent)
    );
}

#[test]
fn a_burned_observe_is_refused_by_the_decision_and_never_put() {
    let mut w = World::new();
    let set = Set::of(&["a"]);
    w.registry.burn(&set.coords[0]);
    let seen = w
        .registry
        .observe(&Grant::none(), &set.coords[0], &[])
        .expect("an observe");
    assert_eq!(
        decide_publish(OURS, &seen),
        PublishDecision::Refused(Reason::PublishBurned)
    );
}

#[test]
fn a_stage_over_a_burned_coordinate_is_a_duplicate_and_stages_nothing() {
    let mut w = World::new();
    let set = Set::of(&["a", "b"]);
    w.registry.burn(&set.coords[1]);
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    let items = set.items(&blobs);
    assert_eq!(w.registry.stage(&g, &items), Ok(StageAnswer::Duplicate));
    // The set is atomic: its other member was not staged either.
    assert!(!w.registry.is_staged(&set.coords[0]));
    assert!(!w.registry.is_staged(&set.coords[1]));
}

#[test]
fn a_commit_over_a_burned_coordinate_is_a_duplicate_and_publishes_nothing() {
    for staging in [true, false] {
        let clock = ManualClock::at(1_000_000);
        let registry = FakeRegistry::new(&clock, LOCATION);
        let mut w = World::with(
            &clock,
            if staging {
                registry
            } else {
                registry.without_staging()
            },
        );
        let set = Set::of(&["a", "b"]);
        w.registry.burn(&set.coords[1]);
        let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Commit, &set.coords));
        let items = set.items(&blobs);
        assert_eq!(
            w.registry.commit(&g, &items),
            Ok(CommitAnswer::Duplicate),
            "staging {staging}"
        );
        assert_eq!(w.registry.bytes(&set.coords[0]), None, "staging {staging}");
        assert_eq!(w.registry.bytes(&set.coords[1]), None, "staging {staging}");
    }
}

#[test]
fn a_burned_coordinate_holds_no_bytes_and_is_not_returned_by_a_listing() {
    let mut w = World::new();
    let set = Set::of(&["a", "ab"]);
    // One sibling of the name prefix is published, the other burned.
    let (blobs, g) = (set.blobs(), w.grant(GrantPhase::Stage, &set.coords));
    let items = set.items(&blobs);
    assert_eq!(w.registry.stage(&g, &items), Ok(StageAnswer::Staged));
    let g = w.grant(GrantPhase::Commit, &set.coords);
    assert_eq!(w.registry.commit(&g, &items), Ok(CommitAnswer::Committed));
    let burned = ucdk_release_proto::v1::Coordinate {
        version: "9.9.9".to_owned(),
        ..set.coords[0].clone()
    };
    w.registry.burn(&burned);
    let read = w.grant(GrantPhase::Read, std::slice::from_ref(&burned));
    assert_eq!(w.registry.fetch(&read, &burned), Ok(None));
    // The newer number is burned, so the newest held coordinate of the name is the published one.
    let newest = w.registry.newest(&read, "stable", "a").expect("newest");
    let versions: Vec<&str> = newest.iter().map(|(c, _)| c.version.as_str()).collect();
    assert_eq!(versions, ["1.2.3"]);
    let scope = ListScope {
        channel: "stable".to_owned(),
        prefix: "a".to_owned(),
    };
    let listed: Vec<String> = w
        .registry
        .list(&read, &scope)
        .expect("a listing")
        .into_iter()
        .map(|l| l.coordinate.name)
        .collect();
    assert_eq!(listed, ["a", "ab"]);
}

#[test]
fn a_burned_coordinate_cannot_be_yanked() {
    let mut w = World::new();
    let set = Set::of(&["a"]);
    w.registry.burn(&set.coords[0]);
    let g = w.grant(GrantPhase::Commit, &set.coords);
    assert!(matches!(
        w.registry.yank(&g, &set.coords[0], "why"),
        Err(SeamError::Permanent(_))
    ));
    assert!(!w.registry.is_yanked(&set.coords[0]));
}

#[test]
fn the_fake_yanks_one_coordinate_whatever_its_shape() {
    let clock = ManualClock::at(1);
    let shapes = [
        FakeRegistry::new(&clock, LOCATION),
        FakeRegistry::new(&clock, LOCATION).without_staging(),
        FakeRegistry::new(&clock, LOCATION).commit_per_coordinate(),
        FakeRegistry::new(&clock, LOCATION).private(),
    ];
    for registry in shapes {
        assert_eq!(registry.yank_unit(), YankUnit::Coordinate);
    }
}
