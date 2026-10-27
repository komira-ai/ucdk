// SPDX-License-Identifier: Apache-2.0
//! The grant source kit: twelve scenarios over the exchange of a job's identity for a grant that
//! covers one phase of one action.

use std::collections::BTreeMap;

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{
    Anonymous, ArtifactType, Coordinate, Federated, PushIdentity, Repository, Visibility,
};

use super::grant_bounds::{
    an_expiry_is_reported_as_the_issuer_stated_it,
    more_coordinates_than_the_limit_are_refused_before_the_exchange,
};
use super::grant_claims::{
    an_earlier_request_does_not_decide_the_next,
    claims_that_only_resemble_the_declared_are_refused,
    several_pinned_claims_are_each_compared_and_none_may_be_added,
};
use super::grant_exceptional::an_exceptional_run_presents_the_exceptional_subject;
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

/// The identity tokens of a source, as its rig counts them: the CI platform gives a new token on
/// every request, and the issuer accepts each token once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityTokens {
    /// The tokens the source asked the CI platform for.
    pub requested: usize,
    /// The tokens the issuer received in an exchange it answered, whether it granted or refused;
    /// an issuer that could not be reached received none.
    pub spent: usize,
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

    /// How many exchanges the source attempted at the issuer, refused ones and ones the issuer
    /// could not be reached for included. A request the source refused itself, before it asked
    /// the issuer anything, is not one.
    fn exchanges(&self, source: &Self::Source) -> usize;

    /// The identity tokens the source asked the CI platform for and the issuer received.
    fn identity_tokens(&self, source: &Self::Source) -> IdentityTokens;

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
pub(super) fn mixed_coordinates() -> [Coordinate; 2] {
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

/// Coordinates whose texts a careless source would change or cut: upper-case letters, a `/`, a
/// newline, a comma, blanks around a name, and a name that reads as another coordinate once a
/// list is split at a newline. A grant covers each of them exactly as it was asked.
pub(super) fn hostile_coordinates() -> Vec<Coordinate> {
    let text = |text: &str| text.to_owned();
    vec![
        Coordinate {
            channel: text("Stable"),
            version: text("1.0.0-RC1"),
            platform: text("Linux-X86_64"),
            ..coordinate("Tool")
        },
        coordinate("dir/tool"),
        coordinate(" tool "),
        coordinate("tool\nrelease"),
        coordinate("a\nread:*"),
        coordinate("a,b"),
        Coordinate {
            channel: text("beta/x"),
            version: text(" 1.0\n"),
            platform: text("linux x86/64"),
            ..coordinate("t")
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
        for (which, asked) in [
            ("two coordinates", coords.to_vec()),
            ("hostile coordinates", hostile_coordinates()),
        ] {
            let mut source = rig.source(&setup(matching_job()));
            let grant = must(
                source.grant(&request(&repository, phase, &asked)),
                &format!("a {phase:?} grant for {which}"),
            );
            same(
                rig.content(&source, &grant),
                GrantContent {
                    scope: scope.to_owned(),
                    repository: LOCATION.to_owned(),
                    coordinates: asked,
                },
                &format!("what a {phase:?} grant for {which} covers"),
            );
        }
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

/// An exchange the issuer refuses with `fault` is reported as that refusal, in every phase, and the
/// next request of the source is exchanged again, with an identity token of its own: a token is
/// accepted once, whether the issuer granted or refused on it.
fn a_used_identity_token_is_refused_the_second_time<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    for phase in PHASES {
        for fault in [GrantFault::Used, GrantFault::Expired] {
            let mut source = rig.source(&setup(matching_job()));
            must(
                source.grant(&request(&repository, phase, &coords)),
                &format!("the first {phase:?} grant"),
            );
            rig.refuse_next(&mut source, phase, fault);
            let second = must_err(
                source.grant(&request(&repository, phase, &coords)),
                &format!("a {phase:?} grant on an identity token refused as {fault:?}"),
            );
            same(
                second,
                SeamError::GrantRefused(fault),
                &format!("the second {phase:?} exchange, refused as {fault:?}"),
            );
            must(
                source.grant(&request(&repository, phase, &coords)),
                &format!("a {phase:?} grant after a refusal as {fault:?}"),
            );
            same(
                rig.exchanges(&source),
                3,
                &format!("the exchanges of three {phase:?} requests, one refused as {fault:?}"),
            );
            same(
                rig.identity_tokens(&source),
                IdentityTokens {
                    requested: 3,
                    spent: 3,
                },
                &format!(
                    "the identity tokens of three {phase:?} exchanges, one refused as {fault:?}"
                ),
            );
        }
    }
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
        // A grant that has not expired is not reused: the same request is a new exchange.
        must(
            source.grant(&request(&repository, phase, &coords)),
            &format!("the same {phase:?} request again, while the grant is good"),
        );
        same(
            rig.exchanges(&source),
            2,
            &format!("the exchanges for a {phase:?} request asked twice"),
        );
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
            3,
            &format!("the exchanges for three {phase:?} grants"),
        );
    }
}

fn a_missing_scope_is_denied<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    for phase in PHASES {
        let mut source = rig.source(&setup(matching_job()));
        rig.refuse_next(&mut source, phase, GrantFault::Denied);
        let refusal = must_err(
            source.grant(&request(&repository, phase, &coords)),
            &format!("a {phase:?} grant for an identity without the scope"),
        );
        same(
            refusal,
            SeamError::GrantRefused(GrantFault::Denied),
            &format!("a missing {phase:?} scope"),
        );
        for other in PHASES {
            must(
                source.grant(&request(&repository, other, &coords)),
                &format!("a {other:?} grant for the same identity after a missing {phase:?} scope"),
            );
        }
    }
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
    let hostile = hostile_coordinates();
    let mut source = rig.source(&setup(matching_job()));
    let grant = must(
        source.grant(&request(&repository, GrantPhase::Read, &hostile)),
        "a read grant for hostile coordinates",
    );
    same(
        rig.content(&source, &grant),
        GrantContent {
            scope: READ_SCOPE.to_owned(),
            repository: LOCATION.to_owned(),
            coordinates: hostile,
        },
        "what a read grant for hostile coordinates covers",
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
    for phase in PHASES {
        let mut source = rig.source(&setup(matching_job()));
        rig.refuse_next(&mut source, phase, GrantFault::Unreachable);
        let first = must_err(
            source.grant(&request(&repository, phase, &coords)),
            &format!("a {phase:?} grant from an issuer that cannot be reached"),
        );
        same(
            first,
            SeamError::GrantRefused(GrantFault::Unreachable),
            &format!("the unreachable issuer, for a {phase:?} grant"),
        );
        same(
            rig.exchanges(&source),
            1,
            &format!("the exchanges of a {phase:?} grant the issuer could not be reached for"),
        );
        same(
            rig.identity_tokens(&source).spent,
            0,
            &format!("the identity tokens an unreachable issuer received for a {phase:?} grant"),
        );
        must(
            source.grant(&request(&repository, phase, &coords)),
            &format!("the {phase:?} grant once the issuer answers, on a token nothing spent"),
        );
        same(
            rig.exchanges(&source),
            2,
            &format!("the exchanges of a {phase:?} grant asked again once the issuer answers"),
        );
        same(
            rig.identity_tokens(&source).spent,
            1,
            &format!("the identity tokens spent on a {phase:?} grant once the issuer answers"),
        );
    }
}
