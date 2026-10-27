// SPDX-License-Identifier: Apache-2.0
//! The scenario of the grant source kit about an exceptional run: it presents the exceptional
//! subject and environment, and the grant it is given has the terms of any other grant.

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_proto::v1::{Coordinate, Reason, Repository};

use super::grant_bounds::{lists_over_the_limit, scope_of};
use super::grant_source::{
    GrantContent, GrantSetup, GrantSourceRig, JobClaims, LOCATION, PHASES, declaration, denied,
    exceptional_job, federated_repository, hostile_coordinates, matching_job, mixed_coordinates,
    request, setup,
};
use super::{coordinate, must, must_err, same};
use crate::{GrantRequest, GrantSource, SeamError};

/// A request of an exceptional run.
fn exceptional_request<'a>(
    repository: &'a Repository,
    phase: GrantPhase,
    coordinates: &'a [Coordinate],
) -> GrantRequest<'a> {
    GrantRequest {
        exceptional: true,
        ..request(repository, phase, coordinates)
    }
}

pub(super) fn an_exceptional_run_presents_the_exceptional_subject<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    let ask = |job: JobClaims, as_exceptional: bool, phase: GrantPhase| {
        let mut source = rig.source(&setup(job));
        let req = GrantRequest {
            exceptional: as_exceptional,
            ..request(&repository, phase, &coords)
        };
        let answer = source.grant(&req);
        (answer.map(|g| g.is_none()), rig.exchanges(&source))
    };
    let refused = (Err(SeamError::GrantRefused(GrantFault::Denied)), 0);
    let exceptional = exceptional_job();
    for phase in PHASES {
        same(
            ask(exceptional.clone(), true, phase),
            (Ok(false), 1),
            &format!("an exceptional job asking as exceptional for a {phase:?} grant"),
        );
        same(
            ask(matching_job(), true, phase),
            refused.clone(),
            &format!("an ordinary job asking as exceptional for a {phase:?} grant"),
        );
        same(
            ask(exceptional.clone(), false, phase),
            refused.clone(),
            &format!("an exceptional job asking as ordinary for a {phase:?} grant"),
        );
    }
    // The subject and the environment of the two pairs are not interchangeable: a job that
    // presents one of each is neither run, whichever it is asked as.
    let crossed = [
        (
            "the ordinary subject and the exceptional environment",
            JobClaims {
                environment: exceptional.environment.clone(),
                ..matching_job()
            },
        ),
        (
            "the exceptional subject and the ordinary environment",
            JobClaims {
                environment: matching_job().environment,
                ..exceptional.clone()
            },
        ),
    ];
    for phase in PHASES {
        for (which, job) in &crossed {
            for as_exceptional in [false, true] {
                same(
                    ask(job.clone(), as_exceptional, phase),
                    refused.clone(),
                    &format!(
                        "a job with {which} asking as exceptional {as_exceptional} for a {phase:?} grant"
                    ),
                );
            }
        }
    }
    // A declaration that names no exceptional subject and no exceptional environment admits no
    // exceptional run, whoever asks: an exceptional run is not an ordinary one in that case, and
    // a job with no subject and no environment is not the run that is not named.
    let undeclared = declaration(LOCATION, |declared| {
        declared.exceptional_subject.clear();
        declared.exceptional_environment.clear();
    });
    let nameless = JobClaims {
        subject: String::new(),
        environment: String::new(),
        ..matching_job()
    };
    for phase in PHASES {
        for (who, job) in [
            ("an ordinary job", matching_job()),
            ("a job with no subject and no environment", nameless.clone()),
        ] {
            let mut source = rig.source(&setup(job));
            let refusal = must_err(
                source.grant(&exceptional_request(&undeclared, phase, &coords)),
                &format!("an exceptional {phase:?} grant where none is declared, for {who}"),
            );
            same(
                denied(&refusal),
                true,
                &format!(
                    "an exceptional {phase:?} grant is Denied where none is declared, for {who}"
                ),
            );
            same(rig.exchanges(&source), 0, "the exchanges of that refusal");
        }
    }
    // One source serves an exceptional run and an ordinary request one after the other, and
    // having been asked as exceptional does not make the next request exceptional.
    let mut source = rig.source(&setup(exceptional.clone()));
    for phase in PHASES {
        for (as_exceptional, want) in [
            (true, Ok(false)),
            (false, refused.0.clone()),
            (true, Ok(false)),
        ] {
            let req = GrantRequest {
                exceptional: as_exceptional,
                ..request(&repository, phase, &coords)
            };
            same(
                source.grant(&req).map(|g| g.is_none()),
                want,
                &format!(
                    "a {phase:?} grant of an exceptional job asked as exceptional {as_exceptional}, one source throughout"
                ),
            );
        }
    }
    same(
        rig.exchanges(&source),
        6,
        "the exchanges of two grants each phase",
    );
    an_exceptional_grant_has_the_terms_of_any_other(rig);
}

/// A grant to an exceptional run is held to what every other grant is: the scope of its phase, the
/// location, exactly the coordinates asked, the expiry the issuer stated and the limit.
fn an_exceptional_grant_has_the_terms_of_any_other<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    for phase in PHASES {
        for (which, asked) in [
            ("two coordinates", mixed_coordinates().to_vec()),
            ("hostile coordinates", hostile_coordinates()),
        ] {
            let mut source = rig.source(&GrantSetup {
                lifetime_ms: 60_000,
                issuer_offset_ms: 30_000,
                ..setup(exceptional_job())
            });
            let stated = rig.now_ms().saturating_add(90_000);
            let grant = must(
                source.grant(&exceptional_request(&repository, phase, &asked)),
                &format!("an exceptional {phase:?} grant for {which}"),
            );
            same(
                rig.content(&source, &grant),
                GrantContent {
                    scope: scope_of(phase).to_owned(),
                    repository: LOCATION.to_owned(),
                    coordinates: asked,
                },
                &format!("what an exceptional {phase:?} grant for {which} covers"),
            );
            same(
                grant.expires_ms(),
                Some(stated),
                &format!("the expiry of an exceptional {phase:?} grant for {which}"),
            );
        }
        let limited = || {
            rig.source(&GrantSetup {
                limit: 2,
                ..setup(exceptional_job())
            })
        };
        let mut source = limited();
        must(
            source.grant(&exceptional_request(
                &repository,
                phase,
                &mixed_coordinates(),
            )),
            &format!("an exceptional {phase:?} grant at the limit"),
        );
        for (which, over) in lists_over_the_limit(2) {
            let mut source = limited();
            let refusal = must_err(
                source.grant(&exceptional_request(&repository, phase, &over)),
                &format!("an exceptional {phase:?} grant over the limit, {which}"),
            );
            same(
                refusal,
                SeamError::Refused(Reason::Credential),
                &format!("the refusal of an exceptional {phase:?} grant over the limit, {which}"),
            );
            same(
                rig.exchanges(&source),
                0,
                &format!("the exchanges of an exceptional {phase:?} grant over the limit, {which}"),
            );
        }
    }
}
