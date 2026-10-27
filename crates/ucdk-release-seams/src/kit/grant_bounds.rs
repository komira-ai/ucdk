// SPDX-License-Identifier: Apache-2.0
//! The scenarios of the grant source kit about the bounds of a grant: the expiry the issuer states
//! and the most coordinates one grant may cover.

use ucdk_release::codec::GrantPhase;
use ucdk_release_proto::v1::{Coordinate, Reason};

use super::grant_source::{
    COMMIT_SCOPE, GrantSetup, GrantSourceRig, PHASES, READ_SCOPE, STAGE_SCOPE,
    federated_repository, hostile_coordinates, matching_job, request, setup,
};
use super::{coordinate, must, must_err, same};
use crate::{GrantSource, SeamError};

/// Lists of one coordinate more than `limit`, each of a kind that a source which counts
/// coordinates by anything but their number would take for no more than `limit`: names that all
/// differ, one name with different versions, and one coordinate repeated.
pub(super) fn lists_over_the_limit(limit: usize) -> Vec<(&'static str, Vec<Coordinate>)> {
    let names: Vec<Coordinate> = (0..=limit)
        .map(|at| coordinate(&format!("n{at}")))
        .collect();
    let versions: Vec<Coordinate> = (0..=limit)
        .map(|at| Coordinate {
            version: format!("1.0.{at}"),
            ..coordinate("a")
        })
        .collect();
    let repeated: Vec<Coordinate> = (0..=limit)
        .map(|at| coordinate(if at == 0 { "a" } else { "b" }))
        .collect();
    let among_others: Vec<Coordinate> = (0..=limit)
        .map(|at| coordinate(&format!("n{}", at.min(limit - 1))))
        .collect();
    vec![
        ("names that all differ", names),
        ("one name with different versions", versions),
        ("coordinates that repeat", repeated),
        (
            "the last coordinate a repeat of the one before",
            among_others,
        ),
    ]
}

pub(super) fn an_expiry_is_reported_as_the_issuer_stated_it<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    // The clock is not at a whole second, as it is not once anything has run: an expiry that is
    // rounded to a second is not the one stated.
    rig.advance(437);
    for phase in PHASES {
        // A short, a middling and a long lifetime, so none is capped or stretched to a constant.
        for lifetime in [60_000_i64, 600_000, 7_200_000] {
            // The issuer's clock ahead and behind; behind by the lifetime, so that the grant is
            // stated to expire now; and behind by more, so that it is stated to have expired
            // already, a moment or a minute and a half ago.
            for offset in [
                30_000,
                -30_000,
                -lifetime,
                -lifetime - 1,
                -lifetime - 90_000,
            ] {
                let mut source = rig.source(&GrantSetup {
                    lifetime_ms: lifetime.unsigned_abs(),
                    issuer_offset_ms: offset,
                    ..setup(matching_job())
                });
                let grant = must(
                    source.grant(&request(&repository, phase, &coords)),
                    &format!("a {phase:?} grant"),
                );
                let stated = rig.now_ms().saturating_add_signed(lifetime + offset);
                same(
                    grant.expires_ms(),
                    Some(stated),
                    &format!(
                        "the expiry of a {phase:?} grant that lives {lifetime} ms from an issuer {offset} ms off"
                    ),
                );
            }
        }
    }
}

pub(super) fn more_coordinates_than_the_limit_are_refused_before_the_exchange<R: GrantSourceRig>(
    rig: &R,
) {
    let repository = federated_repository();
    for phase in PHASES {
        let mut source = rig.source(&GrantSetup {
            limit: 2,
            ..setup(matching_job())
        });
        let two = [coordinate("a"), coordinate("b")];
        must(
            source.grant(&request(&repository, phase, &two)),
            &format!("a {phase:?} grant at the limit"),
        );
        let three = [coordinate("a"), coordinate("b"), coordinate("c")];
        let refusal = must_err(
            source.grant(&request(&repository, phase, &three)),
            &format!("a {phase:?} grant over the limit"),
        );
        same(
            refusal,
            SeamError::Refused(Reason::Credential),
            &format!("the refusal of a {phase:?} grant over the limit"),
        );
        same(
            rig.exchanges(&source),
            1,
            &format!("the exchanges of {phase:?} grants: only the one at the limit"),
        );
        // The limit is of one grant, not of the source: what was granted before does not count.
        for later in [&two[..1], &two[..]] {
            must(
                source.grant(&request(&repository, phase, later)),
                &format!("a {phase:?} grant within the limit after others"),
            );
        }
        same(
            rig.exchanges(&source),
            3,
            &format!("the exchanges of {phase:?} grants within the limit"),
        );
        // The limit counts coordinates, not names and not the coordinates left once the equal
        // ones are taken together: each of these is one over, and none is exchanged.
        for (which, over) in lists_over_the_limit(2) {
            let mut source = rig.source(&GrantSetup {
                limit: 2,
                ..setup(matching_job())
            });
            let refusal = must_err(
                source.grant(&request(&repository, phase, &over)),
                &format!("a {phase:?} grant for three coordinates, {which}"),
            );
            same(
                refusal,
                SeamError::Refused(Reason::Credential),
                &format!("the refusal of a {phase:?} grant for three coordinates, {which}"),
            );
            same(
                rig.exchanges(&source),
                0,
                &format!("the exchanges of a {phase:?} grant for three coordinates, {which}"),
            );
        }
        // At the limit, whatever the texts of the coordinates are, the grant covers them all.
        let hostile = hostile_coordinates();
        let mut source = rig.source(&GrantSetup {
            limit: hostile.len(),
            ..setup(matching_job())
        });
        let grant = must(
            source.grant(&request(&repository, phase, &hostile)),
            &format!("a {phase:?} grant at the limit for hostile coordinates"),
        );
        same(
            rig.content(&source, &grant).coordinates,
            hostile,
            &format!("the coordinates a {phase:?} grant at the limit covers"),
        );
    }
}

/// The scope the declaration of the kit names for `phase`.
pub(super) fn scope_of(phase: GrantPhase) -> &'static str {
    match phase {
        GrantPhase::Stage => STAGE_SCOPE,
        GrantPhase::Commit => COMMIT_SCOPE,
        GrantPhase::Read => READ_SCOPE,
    }
}
