// SPDX-License-Identifier: Apache-2.0
//! The grant source kit: twelve scenarios over the exchange of a job's identity for a grant that
//! covers one phase of one action.

use std::collections::BTreeMap;
use std::fmt::{Debug, Display};
use std::marker::PhantomData;

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{
    Anonymous, ArtifactType, Coordinate, Federated, PushIdentity, Reason, Repository, Visibility,
};

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
    Repository {
        r#type: ArtifactType::File as i32,
        location: LOCATION.to_owned(),
        visibility: Visibility::Private as i32,
        push: Some(PushIdentity {
            kind: Some(Kind::Federated(Federated {
                subject: "release".to_owned(),
                audience: "registry".to_owned(),
                exceptional_subject: "exceptional".to_owned(),
                environment: "publish".to_owned(),
                exceptional_environment: "publish-exceptional".to_owned(),
                pinned_claims: BTreeMap::from([("ref".to_owned(), "main".to_owned())]),
                stage_scope: STAGE_SCOPE.to_owned(),
                commit_scope: COMMIT_SCOPE.to_owned(),
                read_scope: READ_SCOPE.to_owned(),
            })),
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

fn setup(job: JobClaims) -> GrantSetup {
    GrantSetup {
        job,
        limit: usize::MAX,
        lifetime_ms: 600_000,
        issuer_offset_ms: 0,
    }
}

fn request<'a>(
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

fn denied(error: &SeamError) -> bool {
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
    let mut source = rig.source(&setup(matching_job()));
    let coords = [coordinate("a")];
    let grant = must(
        source.grant(&request(&anonymous, GrantPhase::Commit, &coords)),
        "grant",
    );
    same(
        grant.is_none(),
        true,
        "an anonymous identity is answered with no grant",
    );
    same(
        rig.exchanges(&source),
        0,
        "the exchanges an anonymous identity made",
    );
}

fn a_grant_covers_exactly_the_coordinates_asked<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let mut source = rig.source(&setup(matching_job()));
    let coords = [coordinate("a"), coordinate("b")];
    let grant = must(
        source.grant(&request(&repository, GrantPhase::Stage, &coords)),
        "grant",
    );
    same(
        rig.content(&source, &grant),
        GrantContent {
            scope: STAGE_SCOPE.to_owned(),
            repository: LOCATION.to_owned(),
            coordinates: coords.to_vec(),
        },
        "what a stage grant covers",
    );
}

fn a_differing_claim_is_refused_before_any_exchange<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    let job = matching_job();
    let differing = [
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
            "pinned claim",
            JobClaims {
                pinned: BTreeMap::from([("ref".to_owned(), "topic".to_owned())]),
                ..job
            },
        ),
    ];
    for (which, claims) in differing {
        let mut source = rig.source(&setup(claims));
        let refusal = must_err(
            source.grant(&request(&repository, GrantPhase::Stage, &coords)),
            &format!("a grant for a job whose {which} differs"),
        );
        same(
            denied(&refusal),
            true,
            &format!("a differing {which} is Denied"),
        );
        same(
            rig.exchanges(&source),
            0,
            &format!("the exchanges a differing {which} spent"),
        );
    }
}

fn an_exceptional_run_presents_the_exceptional_subject<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let coords = [coordinate("a")];
    let ask = |job: JobClaims, as_exceptional: bool| {
        let mut source = rig.source(&setup(job));
        let req = GrantRequest {
            exceptional: as_exceptional,
            ..request(&repository, GrantPhase::Stage, &coords)
        };
        let answer = source.grant(&req);
        (answer.map(|g| g.is_none()), rig.exchanges(&source))
    };
    same(
        ask(exceptional_job(), true),
        (Ok(false), 1),
        "an exceptional job asking as exceptional",
    );
    same(
        ask(matching_job(), true),
        (Err(SeamError::GrantRefused(GrantFault::Denied)), 0),
        "an ordinary job asking as exceptional",
    );
    same(
        ask(exceptional_job(), false),
        (Err(SeamError::GrantRefused(GrantFault::Denied)), 0),
        "an exceptional job asking as ordinary",
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
    let mut source = rig.source(&setup(matching_job()));
    let first = must(
        source.grant(&request(&repository, GrantPhase::Stage, &coords)),
        "grant",
    );
    let first_expiry = first.expires_ms().unwrap_or_default();
    rig.advance(600_001);
    let second = must(
        source.grant(&request(&repository, GrantPhase::Stage, &coords)),
        "grant",
    );
    let second_expiry = second.expires_ms().unwrap_or_default();
    same(
        second_expiry > first_expiry,
        true,
        "the new grant expires later than the old",
    );
    same(
        second_expiry > rig.now_ms(),
        true,
        "the new grant has not expired",
    );
    same(rig.exchanges(&source), 2, "the exchanges for two grants");
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
    let coords = [coordinate("a"), coordinate("b")];
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
    for offset in [30_000_i64, -30_000] {
        let mut source = rig.source(&GrantSetup {
            lifetime_ms: 60_000,
            issuer_offset_ms: offset,
            ..setup(matching_job())
        });
        let grant = must(
            source.grant(&request(&repository, GrantPhase::Stage, &coords)),
            "grant",
        );
        let stated = rig.now_ms().saturating_add_signed(60_000 + offset);
        same(
            grant.expires_ms(),
            Some(stated),
            &format!("the expiry of a grant from an issuer {offset} ms off"),
        );
    }
}

fn more_coordinates_than_the_limit_are_refused_before_the_exchange<R: GrantSourceRig>(rig: &R) {
    let repository = federated_repository();
    let mut source = rig.source(&GrantSetup {
        limit: 2,
        ..setup(matching_job())
    });
    let two = [coordinate("a"), coordinate("b")];
    must(
        source.grant(&request(&repository, GrantPhase::Stage, &two)),
        "a grant at the limit",
    );
    let three = [coordinate("a"), coordinate("b"), coordinate("c")];
    let refusal = must_err(
        source.grant(&request(&repository, GrantPhase::Stage, &three)),
        "a grant over the limit",
    );
    same(
        refusal,
        SeamError::Refused(Reason::Credential),
        "the refusal over the limit",
    );
    same(
        rig.exchanges(&source),
        1,
        "the exchanges: only the one at the limit",
    );
}

/// Whether a type has a trait, found without naming it as a bound: the inherent constant is
/// there only when the bound holds, and the trait's default stands in when it does not.
struct Probe<T>(PhantomData<T>);

trait Lacks {
    const HAS: bool = false;
}

impl<T> Lacks for Probe<T> {}

// The constants below are read only for a type that has the trait; `Grant` has none of them,
// which is what the scenario holds, so they are never read here.
#[allow(dead_code)]
impl<T: Debug> Probe<T> {
    const HAS: bool = true;
}

struct DisplayProbe<T>(PhantomData<T>);

impl<T> Lacks for DisplayProbe<T> {}

#[allow(dead_code)]
impl<T: Display> DisplayProbe<T> {
    const HAS: bool = true;
}

struct CloneProbe<T>(PhantomData<T>);

impl<T> Lacks for CloneProbe<T> {}

#[allow(dead_code)]
impl<T: Clone> CloneProbe<T> {
    const HAS: bool = true;
}

fn a_grant_is_never_printed_cloned_or_logged<R: GrantSourceRig>(rig: &R) {
    same(<Probe<Grant>>::HAS, false, "Grant implements Debug");
    same(
        <DisplayProbe<Grant>>::HAS,
        false,
        "Grant implements Display",
    );
    same(<CloneProbe<Grant>>::HAS, false, "Grant implements Clone");
    let repository = federated_repository();
    let coords = [coordinate("a")];
    let mut source = rig.source(&setup(matching_job()));
    let grant = must(
        source.grant(&request(&repository, GrantPhase::Stage, &coords)),
        "grant",
    );
    let secret = grant.secret();
    same(
        secret.is_empty(),
        false,
        "a grant with no secret proves nothing",
    );
    let log = rig.log(&source);
    same(
        log.windows(secret.len()).any(|window| window == secret),
        false,
        "the secret of a grant in what the issuer recorded",
    );
}
