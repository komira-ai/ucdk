// SPDX-License-Identifier: Apache-2.0
//! The grant source kit: twelve scenarios over the exchange of a job's identity for a grant that
//! covers one phase of one action.

use std::collections::BTreeMap;

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{
    Anonymous, ArtifactType, Coordinate, Federated, PushIdentity, Reason, Repository, Visibility,
};

use super::grant_claims::{
    an_earlier_request_does_not_decide_the_next,
    claims_that_only_resemble_the_declared_are_refused,
    several_pinned_claims_are_each_compared_and_none_may_be_added,
};
use super::grant_secret::a_grant_is_never_printed_cloned_or_logged;
use super::{Report, Scenario, coordinate, must, must_err, run_scenarios, same};
use crate::{Grant, GrantRequest, GrantSource, SeamError};

/// The claims of the job that asks: what its identity token says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobClaims {
    pub subject: String,
    pub audience: String,
    /// The CI environment the job runs in.
    pub environment: String,
    pub pinned: BTreeMap<String, String>,
}

/// The issuer a scenario wants: the job it serves and how it behaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantSetup {
    pub job: JobClaims,
    /// The most coordinates one grant may cover.
    pub limit: usize,
    /// How long a grant it issues lives.
    pub lifetime_ms: u64,
    /// The expiry it states is on a clock this many milliseconds ahead of the rig's (behind,
    /// when negative).
    pub issuer_offset_ms: i64,
}

/// What a grant says it covers, as the registry would read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantContent {
    pub scope: String,
    /// The location of the repository the grant is for.
    pub repository: String,
    pub coordinates: Vec<Coordinate>,
}

/// What an implementation provides: an issuer for a job, the clock it shares with the scenarios,
/// and the few things only the implementation can see or arrange.
pub trait GrantSourceRig {
    type Source: GrantSource;

    /// An issuer for the job and the behaviour of `setup`.
    fn source(&self, setup: &GrantSetup) -> Self::Source;

    /// The clock the issuer reads, in milliseconds.
    fn now_ms(&self) -> u64;

    /// Moves that clock forward.
    fn advance(&self, ms: u64);

    /// How many exchanges reached the issuer, refused ones included.
    fn exchanges(&self, source: &Self::Source) -> usize;

    /// What `grant` covers, read the way a registry reads it.
    fn content(&self, source: &Self::Source, grant: &Grant) -> GrantContent;

    /// The issuer's next answer to a request of `phase` is a refusal with `fault`: the
    /// identity token was already used, the issuer cannot be reached, the identity may not have
    /// that scope.
    fn refuse_next(&self, source: &mut Self::Source, phase: GrantPhase, fault: GrantFault);

    /// Everything the issuer and the rig recorded, as bytes: a grant's secret is in none of it.
    fn log(&self, source: &Self::Source) -> Vec<u8>;
}

/// The location of the repository [`federated_repository`] names.
pub const LOCATION: &str = "registry/files";
pub const STAGE_SCOPE: &str = "stage";
pub const COMMIT_SCOPE: &str = "commit";
pub const READ_SCOPE: &str = "read";

/// A repository whose push identity is federated, with three distinct scope names.
pub fn federated_repository() -> Repository {
    declaration(LOCATION, |_| {})
}

/// The repository of [`federated_repository`] at `location`, its declaration changed by `change`.
pub(super) fn declaration(location: &str, change: impl FnOnce(&mut Federated)) -> Repository {
    let mut declared = Federated {
        subject: "release".to_owned(),
        audience: "registry".to_owned(),
        exceptional_subject: "exceptional".to_owned(),
        environment: "publish".to_owned(),
        exceptional_environment: "publish-exceptional".to_owned(),
        pinned_claims: BTreeMap::from([("ref".to_owned(), "main".to_owned())]),
        stage_scope: STAGE_SCOPE.to_owned(),
        commit_scope: COMMIT_SCOPE.to_owned(),
        read_scope: READ_SCOPE.to_owned(),
    };
    change(&mut declared);
    Repository {
        r#type: ArtifactType::File as i32,
        location: location.to_owned(),
        visibility: Visibility::Private as i32,
        push: Some(PushIdentity {
            kind: Some(Kind::Federated(declared)),
        }),
    }
}

/// The claims of a job the declaration of [`federated_repository`] admits.
pub fn matching_job() -> JobClaims {
    JobClaims {
        subject: "release".to_owned(),
        audience: "registry".to_owned(),
        environment: "publish".to_owned(),
        pinned: BTreeMap::from([("ref".to_owned(), "main".to_owned())]),
    }
}

/// The claims of a job that runs as the exceptional subject.
pub fn exceptional_job() -> JobClaims {
    JobClaims {
        subject: "exceptional".to_owned(),
        environment: "publish-exceptional".to_owned(),
        ..matching_job()
    }
}

pub(super) fn setup(job: JobClaims) -> GrantSetup {
    GrantSetup {
        job,
        limit: usize::MAX,
        lifetime_ms: 600_000,
        issuer_offset_ms: 0,
    }
}

pub(super) fn request<'a>(
    repository: &'a Repository,
    phase: GrantPhase,
    coordinates: &'a [Coordinate],
) -> GrantRequest<'a> {
    GrantRequest {
        repository,
        phase,
        coordinates,
        exceptional: false,
    }
}

pub(super) fn denied(error: &SeamError) -> bool {
    *error == SeamError::GrantRefused(GrantFault::Denied)
}

pub fn run_grant_source_kit<R: GrantSourceRig>(rig: &R) -> Report {
    run_scenarios(rig, &grant_source_scenarios())
}

pub fn grant_source_scenarios<R: GrantSourceRig>() -> Vec<Scenario<R>> {
    vec![
        Scenario::new(
            "an_anonymous_identity_needs_no_exchange",
            an_anonymous_identity_needs_no_exchange::<R>,
        ),
        Scenario::new(
            "a_grant_covers_exactly_the_coordinates_asked",
            a_grant_covers_exactly_the_coordinates_asked::<R>,
        ),
        Scenario::new(
            "a_differing_claim_is_refused_before_any_exchange",
            a_differing_claim_is_refused_before_any_exchange::<R>,
        ),
        Scenario::new(
            "an_exceptional_run_presents_the_exceptional_subject",
            an_exceptional_run_presents_the_exceptional_subject::<R>,
        ),
        Scenario::new(
            "a_used_identity_token_is_refused_the_second_time",
            a_used_identity_token_is_refused_the_second_time::<R>,
        ),
        Scenario::new(
            "an_exchange_after_expiry_gives_a_new_grant",
            an_exchange_after_expiry_gives_a_new_grant::<R>,
        ),
        Scenario::new("a_missing_scope_is_denied", a_missing_scope_is_denied::<R>),
        Scenario::new(
            "a_read_grant_carries_the_read_scope_and_covers_exactly_the_coordinates_asked",
            a_read_grant_carries_the_read_scope_and_covers_exactly_the_coordinates_asked::<R>,
        ),
        Scenario::new(
            "an_unreachable_issuer_is_unreachable_and_spends_nothing",
            an_unreachable_issuer_is_unreachable_and_spends_nothing::<R>,
        ),
        Scenario::new(
            "an_expiry_is_reported_as_the_issuer_stated_it",
            an_expiry_is_reported_as_the_issuer_stated_it::<R>,
        ),
        Scenario::new(
            "more_coordinates_than_the_limit_are_refused_before_the_exchange",
            more_coordinates_than_the_limit_are_refused_before_the_exchange::<R>,
        ),
        Scenario::new(
            "a_grant_is_never_printed_cloned_or_logged",
            a_grant_is_never_printed_cloned_or_logged::<R>,
        ),
    ]
}

fn an_anonymous_identity_needs_no_exchange<R: GrantSourceRig>(rig: &R) {
    let anonymous = Repository {
        push: Some(PushIdentity {
            kind: Some(Kind::Anonymous(Anonymous {})),
        }),
        ..federated_repository()
    };
    let coords = [coordinate("a")];
    let elsewhere = JobClaims {
        subject: "someone".to_owned(),
        ..matching_job()
    };
    // For every phase, as an ordinary or an exceptional run, whatever the job claims: there is
    // nothing to exchange.
    for phase in PHASES {
        for exceptional in [false, true] {
            for job in [matching_job(), elsewhere.clone()] {
                let mut source = rig.source(&setup(job));
                let grant = must(
                    source.grant(&GrantRequest {
                        exceptional,
                        ..request(&anonymous, phase, &coords)
                    }),
                    &format!("a {phase:?} grant, exceptional {exceptional}"),
                );
                same(
                    grant.is_none(),
                    true,
                    &format!("an anonymous identity is answered with no {phase:?} grant"),
                );
                same(
                    rig.exchanges(&source),
                    0,
                    &format!("the exchanges an anonymous identity made for a {phase:?} grant"),
                );
            }
        }
    }
}

/// Every phase a grant is asked for.
pub(super) const PHASES: [GrantPhase; 3] =
    [GrantPhase::Stage, GrantPhase::Commit, GrantPhase::Read];

/// Two coordinates that differ from the kits' default in every field but the name: a grant
/// covers the version, platform, channel and type it is asked for.
fn mixed_coordinates() -> [Coordinate; 2] {
    [
        coordinate("a"),
        Coordinate {
            channel: "beta".to_owned(),
            r#type: ArtifactType::Oci as i32,
            version: "9.9.9".to_owned(),
            platform: "darwin-arm64".to_owned(),
            ..coordinate("b")
        },
    ]
}

/// The declaration of [`federated_repository`] with its location and every scope name changed,
/// and the same claims: the job [`matching_job`] is admitted by both. With `one_scope`, the
/// stage and the commit phase share one scope name.
fn renamed_declaration(one_scope: bool) -> Repository {
    declaration("elsewhere/repo", |declared| {
        declared.stage_scope = "s2".to_owned();
        declared.commit_scope = if one_scope { "s2" } else { "c2" }.to_owned();
        declared.read_scope = "r2".to_owned();
    })
}

/// One source serves many requests, and each grant is of the phase and the repository of its
/// own request, whatever was asked before.
fn one_source_serves_every_request_as_asked<R: GrantSourceRig>(rig: &R) {
    let coords = [coordinate("a")];
    let (first, second) = (federated_repository(), renamed_declaration(false));
    let mut source = rig.source(&setup(matching_job()));
    for (repository, phase, scope, location) in [
        (&first, GrantPhase::Stage, STAGE_SCOPE, LOCATION),
        (&first, GrantPhase::Commit, COMMIT_SCOPE, LOCATION),
        (&second, GrantPhase::Commit, "c2", "elsewhere/repo"),
        (&second, GrantPhase::Stage, "s2", "elsewhere/repo"),
        (&first, GrantPhase::Stage, STAGE_SCOPE, LOCATION),
    ] {
        let grant = must(
            source.grant(&request(repository, phase, &coords)),
            &format!("a {phase:?} grant after other requests of one source"),
        );
        same(
            rig.content(&source, &grant),
            GrantContent {
                scope: scope.to_owned(),
                repository: location.to_owned(),
                coordinates: coords.to_vec(),
            },
            &format!("what a {phase:?} grant of {location} covers after other requests"),
        );
    }
    // A declaration with one scope name for both phases is a declaration like another.
    let shared = renamed_declaration(true);
    for phase in [GrantPhase::Stage, GrantPhase::Commit] {
        let mut source = rig.source(&setup(matching_job()));
        let grant = must(
            source.grant(&request(&shared, phase, &coords)),
            &format!("a {phase:?} grant when both phases have one scope"),
        );
        same(
            rig.content(&source, &grant),
            GrantContent {
                scope: "s2".to_owned(),
                repository: "elsewhere/repo".to_owned(),
                coordinates: coords.to_vec(),
            },
            &format!("what a {phase:?} grant covers when both phases have one scope"),
        );
    }
}

fn a_grant_covers_exactly_the_coordinates_asked<R: GrantSourceRig>(rig: &R) {
    one_source_serves_every_request_as_asked(rig);
    let repository = federated_repository();
    // A request for nothing is refused or covers nothing: never more than was asked.
    for phase in PHASES {
        let mut source = rig.source(&setup(matching_job()));
        let covered = source
            .grant(&request(&repository, phase, &[]))
            .ok()
            .map(|grant| rig.content(&source, &grant).coordinates);
        same(
            covered.unwrap_or_default(),
            Vec::new(),
            &format!("what a {phase:?} grant for no coordinate covers"),
        );
    }
    let coords = mixed_coordinates();
    for (phase, scope) in [
        (GrantPhase::Stage, STAGE_SCOPE),
        (GrantPhase::Commit, COMMIT_SCOPE),
    ] {
        let mut source = rig.source(&setup(matching_job()));
        let grant = must(source.grant(&request(&repository, phase, &coords)), "grant");
        same(
            rig.content(&source, &grant),
            GrantContent {
                scope: scope.to_owned(),
                repository: LOCATION.to_owned(),
                coordinates: coords.to_vec(),
            },
            &format!("what a {phase:?} grant covers"),
        );
    }
    // A declaration that shares no name with the one above: the grant follows the request.
    let (custom, job) = custom_declaration();
    for (phase, scope) in [
        (GrantPhase::Stage, "s-custom"),
        (GrantPhase::Commit, "c-custom"),
    ] {
        let mut source = rig.source(&setup(job.clone()));
        let grant = must(
            source.grant(&request(&custom, phase, &coords)),
            &format!("a {phase:?} grant for another declaration"),
        );
        same(
            rig.content(&source, &grant),
            GrantContent {
                scope: scope.to_owned(),
                repository: "elsewhere/repo".to_owned(),
                coordinates: coords.to_vec(),
            },
            &format!("what a {phase:?} grant covers when the declaration names its own scopes"),
        );
    }
}

/// A declaration that shares nothing with [`federated_repository`]: its location, the names of
/// its scopes and every claim differ. The claims of the job that it admits come with it.
fn custom_declaration() -> (Repository, JobClaims) {
    let claim = |text: &str| text.to_owned();
    let pinned = BTreeMap::from([(claim("branch"), claim("trunk"))]);
    let repository = Repository {
        r#type: ArtifactType::File as i32,
        location: claim("elsewhere/repo"),
        visibility: Visibility::Private as i32,
        push: Some(PushIdentity {
            kind: Some(Kind::Federated(Federated {
                subject: claim("custom-subject"),
                audience: claim("custom-audience"),
                exceptional_subject: claim("custom-exceptional"),
                environment: claim("custom-env"),
                exceptional_environment: claim("custom-env-exceptional"),
                pinned_claims: pinned.clone(),
                stage_scope: claim("s-custom"),
                commit_scope: claim("c-custom"),
                read_scope: claim("r-custom"),
            })),
        }),
    };
    let job = JobClaims {
        subject: claim("custom-subject"),
        audience: claim("custom-audience"),
        environment: claim("custom-env"),
        pinned,
    };
    (repository, job)
}

fn a_differing_claim_is_refused_before_any_exchange<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    let job = matching_job();
    let differing = [
        (
            "subject that holds the declared one",
            JobClaims {
                subject: format!("{}-evil", job.subject),
                ..job.clone()
            },
        ),
        (
            "pinned claims, none presented",
            JobClaims {
                pinned: BTreeMap::new(),
                ..job.clone()
            },
        ),
        (
            "subject",
            JobClaims {
                subject: "someone".to_owned(),
                ..job.clone()
            },
        ),
        (
            "audience",
            JobClaims {
                audience: "elsewhere".to_owned(),
                ..job.clone()
            },
        ),
        (
            "environment",
            JobClaims {
                environment: "preview".to_owned(),
                ..job.clone()
            },
        ),
        (
            "subject in another case",
            JobClaims {
                subject: job.subject.to_uppercase(),
                ..job.clone()
            },
        ),
        (
            "audience in another case",
            JobClaims {
                audience: job.audience.to_uppercase(),
                ..job.clone()
            },
        ),
        (
            "environment in another case",
            JobClaims {
                environment: job.environment.to_uppercase(),
                ..job.clone()
            },
        ),
        (
            "pinned claim value in another case",
            JobClaims {
                pinned: BTreeMap::from([("ref".to_owned(), "MAIN".to_owned())]),
                ..job.clone()
            },
        ),
        (
            "pinned claim name in another case",
            JobClaims {
                pinned: BTreeMap::from([("REF".to_owned(), "main".to_owned())]),
                ..job.clone()
            },
        ),
        (
            "pinned claim",
            JobClaims {
                pinned: BTreeMap::from([("ref".to_owned(), "topic".to_owned())]),
                ..job
            },
        ),
    ];
    for phase in PHASES {
        for (which, claims) in &differing {
            let mut source = rig.source(&setup(claims.clone()));
            let refusal = must_err(
                source.grant(&request(&repository, phase, &coords)),
                &format!("a {phase:?} grant for a job whose {which} differs"),
            );
            same(
                denied(&refusal),
                true,
                &format!("a differing {which} is Denied for a {phase:?} grant"),
            );
            same(
                rig.exchanges(&source),
                0,
                &format!("the exchanges a differing {which} spent on a {phase:?} grant"),
            );
        }
    }
    // An exceptional run presents its own subject and environment and every other claim the
    // same as an ordinary one, and each of them is compared.
    let exceptional = exceptional_job();
    let differing_exceptional = [
        (
            "environment (the ordinary one)",
            JobClaims {
                environment: matching_job().environment,
                ..exceptional.clone()
            },
        ),
        (
            "audience",
            JobClaims {
                audience: "elsewhere".to_owned(),
                ..exceptional.clone()
            },
        ),
        (
            "subject in another case",
            JobClaims {
                subject: exceptional.subject.to_uppercase(),
                ..exceptional.clone()
            },
        ),
        (
            "pinned claim",
            JobClaims {
                pinned: BTreeMap::from([("ref".to_owned(), "topic".to_owned())]),
                ..exceptional
            },
        ),
    ];
    for phase in PHASES {
        for (which, claims) in &differing_exceptional {
            let mut source = rig.source(&setup(claims.clone()));
            let req = GrantRequest {
                exceptional: true,
                ..request(&repository, phase, &coords)
            };
            let refusal = must_err(
                source.grant(&req),
                &format!("a {phase:?} grant for an exceptional run whose {which} differs"),
            );
            same(
                denied(&refusal),
                true,
                &format!(
                    "a differing {which} of an exceptional run is Denied for a {phase:?} grant"
                ),
            );
            same(
                rig.exchanges(&source),
                0,
                &format!("the exchanges a differing {which} of an exceptional run spent"),
            );
        }
    }
    several_pinned_claims_are_each_compared_and_none_may_be_added(rig);
    an_earlier_request_does_not_decide_the_next(rig);
    claims_that_only_resemble_the_declared_are_refused(rig);
}

fn an_exceptional_run_presents_the_exceptional_subject<R: GrantSourceRig>(rig: &R) {
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
    // exceptional run, whoever asks: an exceptional run is not an ordinary one in that case.
    let undeclared = declaration(LOCATION, |declared| {
        declared.exceptional_subject.clear();
        declared.exceptional_environment.clear();
    });
    for phase in PHASES {
        let mut source = rig.source(&setup(matching_job()));
        let req = GrantRequest {
            exceptional: true,
            ..request(&undeclared, phase, &coords)
        };
        let refusal = must_err(
            source.grant(&req),
            &format!("an exceptional {phase:?} grant where none is declared"),
        );
        same(
            denied(&refusal),
            true,
            &format!("an exceptional {phase:?} grant is Denied where none is declared"),
        );
        same(rig.exchanges(&source), 0, "the exchanges of that refusal");
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
}

fn a_used_identity_token_is_refused_the_second_time<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    let mut source = rig.source(&setup(matching_job()));
    must(
        source.grant(&request(&repository, GrantPhase::Stage, &coords)),
        "the first grant",
    );
    rig.refuse_next(&mut source, GrantPhase::Stage, GrantFault::Used);
    let second = must_err(
        source.grant(&request(&repository, GrantPhase::Stage, &coords)),
        "a grant on a used identity token",
    );
    same(
        second,
        SeamError::GrantRefused(GrantFault::Used),
        "the second exchange",
    );
}

fn an_exchange_after_expiry_gives_a_new_grant<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    // Nothing is standing: a grant of any phase expires, and the next exchange gives a new one.
    for phase in PHASES {
        let mut source = rig.source(&setup(matching_job()));
        let first = must(
            source.grant(&request(&repository, phase, &coords)),
            &format!("a {phase:?} grant"),
        );
        let first_expiry = first.expires_ms().unwrap_or_default();
        rig.advance(600_001);
        let second = must(
            source.grant(&request(&repository, phase, &coords)),
            &format!("a {phase:?} grant after the first expired"),
        );
        let second_expiry = second.expires_ms().unwrap_or_default();
        same(
            second_expiry > first_expiry,
            true,
            &format!("the new {phase:?} grant expires later than the old"),
        );
        same(
            second_expiry > rig.now_ms(),
            true,
            &format!("the new {phase:?} grant has not expired"),
        );
        same(
            rig.exchanges(&source),
            2,
            &format!("the exchanges for two {phase:?} grants"),
        );
    }
}

fn a_missing_scope_is_denied<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    let mut source = rig.source(&setup(matching_job()));
    rig.refuse_next(&mut source, GrantPhase::Commit, GrantFault::Denied);
    let refusal = must_err(
        source.grant(&request(&repository, GrantPhase::Commit, &coords)),
        "a commit grant for an identity without the commit scope",
    );
    same(denied(&refusal), true, "a missing scope is Denied");
    must(
        source.grant(&request(&repository, GrantPhase::Stage, &coords)),
        "a stage grant for the same identity",
    );
}

fn a_read_grant_carries_the_read_scope_and_covers_exactly_the_coordinates_asked<
    R: GrantSourceRig,
>(
    rig: &R,
) {
    let repository = federated_repository();
    let mut source = rig.source(&setup(matching_job()));
    let coords = mixed_coordinates();
    let grant = must(
        source.grant(&request(&repository, GrantPhase::Read, &coords)),
        "grant",
    );
    same(
        rig.content(&source, &grant),
        GrantContent {
            scope: READ_SCOPE.to_owned(),
            repository: LOCATION.to_owned(),
            coordinates: coords.to_vec(),
        },
        "what a read grant covers",
    );
    // The scope and the location are those of the declaration the request carries.
    let (custom, job) = custom_declaration();
    let mut source = rig.source(&setup(job));
    let grant = must(
        source.grant(&request(&custom, GrantPhase::Read, &coords)),
        "a read grant for another declaration",
    );
    same(
        rig.content(&source, &grant),
        GrantContent {
            scope: "r-custom".to_owned(),
            repository: "elsewhere/repo".to_owned(),
            coordinates: coords.to_vec(),
        },
        "what a read grant covers when the declaration names its own scopes",
    );
}

fn an_unreachable_issuer_is_unreachable_and_spends_nothing<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    let mut source = rig.source(&setup(matching_job()));
    rig.refuse_next(&mut source, GrantPhase::Stage, GrantFault::Unreachable);
    let first = must_err(
        source.grant(&request(&repository, GrantPhase::Stage, &coords)),
        "a grant from an issuer that cannot be reached",
    );
    same(
        first,
        SeamError::GrantRefused(GrantFault::Unreachable),
        "the unreachable issuer",
    );
    must(
        source.grant(&request(&repository, GrantPhase::Stage, &coords)),
        "the grant once the issuer answers, on an identity token nothing spent",
    );
}

fn an_expiry_is_reported_as_the_issuer_stated_it<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    // The clock is not at a whole second, as it is not once anything has run: an expiry that is
    // rounded to a second is not the one stated.
    rig.advance(437);
    for phase in PHASES {
        for offset in [30_000_i64, -30_000] {
            let mut source = rig.source(&GrantSetup {
                lifetime_ms: 60_000,
                issuer_offset_ms: offset,
                ..setup(matching_job())
            });
            let grant = must(
                source.grant(&request(&repository, phase, &coords)),
                &format!("a {phase:?} grant"),
            );
            let stated = rig.now_ms().saturating_add_signed(60_000 + offset);
            same(
                grant.expires_ms(),
                Some(stated),
                &format!("the expiry of a {phase:?} grant from an issuer {offset} ms off"),
            );
        }
    }
}

fn more_coordinates_than_the_limit_are_refused_before_the_exchange<R: GrantSourceRig>(rig: &R) {
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
    }
}
