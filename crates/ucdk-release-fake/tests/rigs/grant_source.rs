// SPDX-License-Identifier: Apache-2.0
//! The grant-source rig: the fake issuer, and the stubs that break one promise of an issuer.

use std::time::Duration;

use ucdk_core::{Clock, ManualClock};
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_fake::{FakeGrantSource, JobClaims as FakeClaims};
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{Reason, Repository};
use ucdk_release_seams::kit::{
    COMMIT_SCOPE, GrantContent, GrantSetup, GrantSourceRig, JobClaims, LOCATION, READ_SCOPE,
    STAGE_SCOPE, coordinate,
};
use ucdk_release_seams::{Grant, GrantRequest, GrantSource, SeamError};

use super::secret::{framed, texts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flaw {
    /// An anonymous identity is answered with a grant.
    AnonymousGetsAGrant,
    /// A stage grant for two coordinates covers only the first.
    DropsTheLastCoordinate,
    /// The pinned claims of the job are not compared.
    IgnoresPinnedClaims,
    /// The request is never exceptional.
    IgnoresExceptional,
    /// A used identity token is refused as `Denied`.
    UsedIsDenied,
    /// An expired grant is handed out again, as it was.
    ReissuesTheExpiredGrant,
    /// A commit grant for an identity without the scope is issued anyway.
    CommitWithoutScopeIsIssued,
    /// A read grant is issued with the stage scope.
    ReadIsIssuedAsStage,
    /// An unreachable issuer spends the identity token.
    UnreachableSpendsTheToken,
    /// The stated expiry is moved by the offset of the issuer's clock.
    CorrectsTheSkew,
    /// A grant for one coordinate over the limit is issued for the limit.
    AllowsOneOverTheLimit,
    /// The secret of a grant is written into the log.
    LogsTheSecret,
    /// The claims of the job are compared for a stage grant only.
    SkipsClaimsOutsideStage,
    /// The subject of the job need only contain the declared subject.
    ClaimsMatchedBySubstring,
    /// A job with no pinned claim is admitted where claims are pinned.
    NoPinnedClaimIsIssued,
    /// The environment of an exceptional run is not compared.
    ExceptionalIgnoresEnvironment,
    /// The audience and the pinned claims of an exceptional run are not compared.
    ExceptionalIgnoresAudienceAndPinned,
    /// The limit is enforced for a stage grant only.
    LimitIsCheckedForStageOnly,
    /// A commit grant is issued with the stage scope.
    CommitGrantHasTheStageScope,
    /// A commit grant for two coordinates covers the first.
    CommitGrantDropsACoordinate,
    /// The scope and the location of a grant are those of the repository the kit builds, whatever
    /// the declaration of the request names.
    IgnoresTheDeclaredScopesAndLocation,
    /// The claims of the job are compared without regard to case.
    ClaimsMatchedIgnoringCase,
    /// Each coordinate of a request is covered with the version, platform, channel and type the
    /// kits use, whatever the request names.
    NormalizesCoordinateFields,
    /// An exceptional request of a declaration that names no exceptional subject and no
    /// exceptional environment is compared with the ordinary ones.
    ExceptionalFallsBackWhenUndeclared,
    /// Of the pinned claims, only the first one declared is compared.
    PinnedOnlyFirstCompared,
    /// A job that presents every pinned claim declared and more is admitted.
    PinnedSubset,
    /// A commit grant is denied when the declaration has one scope for both phases.
    EqualScopesCommitDenied,
    /// Once an exceptional request was granted, every later request of the source is exceptional.
    StickyExceptional,
    /// Every grant of a source has the phase of the first request it was asked.
    ScopeStickyToFirstRequest,
    /// Every grant of a source is for the repository of the first request it was asked.
    StickyRepository,
    /// The expiry of a grant is rounded down to a whole second.
    ExpiryRoundedToSecond,
    /// The subject of the job need only end with the declared subject.
    SubjectMatchedBySuffix,
    /// The audience of the job need only contain the declared audience.
    AudienceMatchedByContainment,
    /// The environment of the job need only start with the declared environment.
    EnvironmentMatchedByPrefix,
    /// The value of a pinned claim of the job need only contain the declared value.
    PinnedValueMatchedByContainment,
    /// The claims are compared after folding full-width letters, the Kelvin sign and the long s
    /// to the letters they resemble.
    ClaimsMatchedAfterFolding,
    /// The claims are compared after trimming white space.
    ClaimsMatchedAfterTrimming,
    /// An anonymous identity is answered with a grant for a stage or a read grant.
    AnonymousStageOrReadGetsAGrant,
    /// A commit grant and a read grant never expire.
    NonStageGrantsNeverExpire,
    /// The limit counts the coordinates of every grant a source has issued.
    CumulativeLimit,
    /// A request for no coordinate is issued a grant that covers one named `*`.
    EmptyRequestCoversAnything,
    /// The secret of a grant is written into the log reversed.
    LogsTheSecretReversed,
    /// The secret of a grant is written into the log without its first and last two bytes.
    LogsTheMiddleOfTheSecret,
    /// The secret of a grant is written into the log in hex.
    LogsTheSecretInHex,
    /// The claims are compared only for the first repository a source is asked about.
    ClaimsCheckedForTheFirstRepositoryOnly,
    /// A request that was refused and is asked again as it was is issued.
    RepeatOfRefusedIssues,
    /// An exceptional run is admitted on the ordinary subject too.
    ExceptionalAcceptsTheOrdinarySubject,
    /// An ordinary run is admitted on the exceptional subject too.
    OrdinaryAcceptsTheExceptionalSubject,
    /// An ordinary run is admitted on the exceptional environment too.
    OrdinaryAcceptsTheExceptionalEnvironment,
    /// The claims are compared after removing zero-width characters.
    ClaimsMatchedWithoutInvisibleCharacters,
    /// The claims are compared up to the first NUL.
    ClaimsMatchedUpToANul,
    /// The secret of a commit grant is written into the log.
    LogsTheCommitSecret,
    /// The secret of an exceptional grant is written into the log.
    LogsTheExceptionalSecret,
    /// The secret of a grant is written into the log in base64.
    LogsTheSecretInBase64,
    /// The secret of a grant is written into the log with a space after each byte.
    LogsTheSecretSpaced,
    /// The first two fifths of the secret of a grant are written into the log.
    LogsThePrefixOfTheSecret,
    /// The secret of every grant of a source but its first is written into the log.
    LogsTheSecretFromTheSecondGrant,
    /// The secret of every grant after a refusal is written into the log.
    LogsTheSecretAfterARefusal,
}

impl Flaw {
    /// Every stub of the issuer.
    pub const ALL: [Self; 58] = [
        Self::AnonymousGetsAGrant,
        Self::DropsTheLastCoordinate,
        Self::IgnoresPinnedClaims,
        Self::IgnoresExceptional,
        Self::UsedIsDenied,
        Self::ReissuesTheExpiredGrant,
        Self::CommitWithoutScopeIsIssued,
        Self::ReadIsIssuedAsStage,
        Self::UnreachableSpendsTheToken,
        Self::CorrectsTheSkew,
        Self::AllowsOneOverTheLimit,
        Self::LogsTheSecret,
        Self::SkipsClaimsOutsideStage,
        Self::ClaimsMatchedBySubstring,
        Self::NoPinnedClaimIsIssued,
        Self::ExceptionalIgnoresEnvironment,
        Self::ExceptionalIgnoresAudienceAndPinned,
        Self::LimitIsCheckedForStageOnly,
        Self::CommitGrantHasTheStageScope,
        Self::CommitGrantDropsACoordinate,
        Self::IgnoresTheDeclaredScopesAndLocation,
        Self::ClaimsMatchedIgnoringCase,
        Self::NormalizesCoordinateFields,
        Self::ExceptionalFallsBackWhenUndeclared,
        Self::PinnedOnlyFirstCompared,
        Self::PinnedSubset,
        Self::EqualScopesCommitDenied,
        Self::StickyExceptional,
        Self::ScopeStickyToFirstRequest,
        Self::StickyRepository,
        Self::ExpiryRoundedToSecond,
        Self::SubjectMatchedBySuffix,
        Self::AudienceMatchedByContainment,
        Self::EnvironmentMatchedByPrefix,
        Self::PinnedValueMatchedByContainment,
        Self::ClaimsMatchedAfterFolding,
        Self::ClaimsMatchedAfterTrimming,
        Self::AnonymousStageOrReadGetsAGrant,
        Self::NonStageGrantsNeverExpire,
        Self::CumulativeLimit,
        Self::EmptyRequestCoversAnything,
        Self::LogsTheSecretReversed,
        Self::LogsTheMiddleOfTheSecret,
        Self::LogsTheSecretInHex,
        Self::ClaimsCheckedForTheFirstRepositoryOnly,
        Self::RepeatOfRefusedIssues,
        Self::ExceptionalAcceptsTheOrdinarySubject,
        Self::OrdinaryAcceptsTheExceptionalSubject,
        Self::OrdinaryAcceptsTheExceptionalEnvironment,
        Self::ClaimsMatchedWithoutInvisibleCharacters,
        Self::ClaimsMatchedUpToANul,
        Self::LogsTheCommitSecret,
        Self::LogsTheExceptionalSecret,
        Self::LogsTheSecretInBase64,
        Self::LogsTheSecretSpaced,
        Self::LogsThePrefixOfTheSecret,
        Self::LogsTheSecretFromTheSecondGrant,
        Self::LogsTheSecretAfterARefusal,
    ];
}

/// The claim of a job a stub compares.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Subject,
    Audience,
    Environment,
    PinnedName,
    PinnedValue,
}

/// A text as a lax issuer folds it before it compares: full-width letters, the Kelvin sign and
/// the long s become the letters they resemble.
fn folded(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{212a}' => 'k',
            '\u{17f}' => 's',
            c => char::from_u32(u32::from(c).wrapping_sub(0xFEE0))
                .filter(char::is_ascii_graphic)
                .unwrap_or(c),
        })
        .collect()
}

/// A text without its zero-width characters.
fn without_invisible(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}' | '\u{2060}'
            )
        })
        .collect()
}

/// A text up to its first NUL.
fn up_to_nul(text: &str) -> &str {
    text.split('\0').next().unwrap_or_default()
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    bytes
        .chunks(3)
        .flat_map(|chunk| {
            let word = chunk.iter().enumerate().fold(0u32, |word, (at, byte)| {
                word | (u32::from(*byte) << (16 - 8 * at))
            });
            (0..4).map(move |at| {
                if at <= chunk.len() {
                    ALPHABET[((word >> (18 - 6 * at)) & 63) as usize] as char
                } else {
                    '='
                }
            })
        })
        .collect()
}

/// What a source was asked when it refused: the repository, the phase, whether the run is
/// exceptional, and the coordinates.
type Asked = (Repository, GrantPhase, bool, Vec<Coordinate>);

pub struct FakeGrantRig {
    clock: ManualClock,
    flaw: Option<Flaw>,
}

impl FakeGrantRig {
    pub fn conforming() -> Self {
        Self {
            clock: ManualClock::at(1_000_000),
            flaw: None,
        }
    }

    pub fn flawed(flaw: Flaw) -> Self {
        Self {
            flaw: Some(flaw),
            ..Self::conforming()
        }
    }
}

pub struct Flawed {
    inner: FakeGrantSource,
    job: JobClaims,
    flaw: Option<Flaw>,
    clock: ManualClock,
    limit: usize,
    offset_ms: i64,
    cached: Option<(Vec<u8>, u64)>,
    unreachable_seen: bool,
    log: Vec<u8>,
    sticky_exceptional: bool,
    first_phase: Option<GrantPhase>,
    first_repository: Option<Repository>,
    covered: usize,
    first_claims: Option<Repository>,
    refused_request: Option<Asked>,
    issued: usize,
    refused: bool,
}

/// The coordinate a registry reads out of the text a grant keeps it under: the channel, the
/// type, the name, the version and the platform, each a text of its own.
fn coordinate_of(key: &str) -> Coordinate {
    let fields = texts(key.as_bytes());
    let field = |at: usize| fields.get(at).cloned().unwrap_or_default();
    Coordinate {
        channel: field(0),
        r#type: field(1).parse().unwrap_or_default(),
        name: field(2),
        version: field(3),
        platform: field(4),
    }
}

impl GrantSourceRig for FakeGrantRig {
    type Source = Flawed;

    fn source(&self, setup: &GrantSetup) -> Flawed {
        let claims = FakeClaims {
            subject: setup.job.subject.clone(),
            audience: setup.job.audience.clone(),
            environment: setup.job.environment.clone(),
            pinned: setup.job.pinned.clone(),
        };
        let inner_limit = if self.flaw == Some(Flaw::LimitIsCheckedForStageOnly) {
            usize::MAX
        } else {
            setup.limit
        };
        let mut inner = FakeGrantSource::new(&self.clock, claims).with_limit(inner_limit);
        inner.lifetime(setup.lifetime_ms);
        inner.issuer_clock_offset(setup.issuer_offset_ms);
        Flawed {
            inner,
            job: setup.job.clone(),
            flaw: self.flaw,
            clock: self.clock.clone(),
            limit: setup.limit,
            offset_ms: setup.issuer_offset_ms,
            cached: None,
            unreachable_seen: false,
            log: Vec::new(),
            sticky_exceptional: false,
            first_phase: None,
            first_repository: None,
            covered: 0,
            first_claims: None,
            refused_request: None,
            issued: 0,
            refused: false,
        }
    }

    fn now_ms(&self) -> u64 {
        self.clock.now_millis()
    }

    fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
    }

    fn exchanges(&self, source: &Flawed) -> usize {
        source.inner.exchanges().len()
    }

    fn content(&self, _source: &Flawed, grant: &Grant) -> GrantContent {
        let mut all = texts(grant.secret()).into_iter();
        let scope = all.next().unwrap_or_default();
        let repository = all.next().unwrap_or_default();
        GrantContent {
            scope,
            repository,
            coordinates: all.map(|key| coordinate_of(&key)).collect(),
        }
    }

    fn refuse_next(&self, source: &mut Flawed, phase: GrantPhase, fault: GrantFault) {
        source.inner.refuse(phase, fault);
    }

    fn log(&self, source: &Flawed) -> Vec<u8> {
        let mut log = format!("{:?}", source.inner.exchanges()).into_bytes();
        log.extend_from_slice(&source.log);
        log
    }
}

impl Flawed {
    fn is(&self, flaw: Flaw) -> bool {
        self.flaw == Some(flaw)
    }

    /// The repository as a lax issuer reads it: the declaration with each claim the stub does
    /// not compare made equal to the job's.
    fn as_a_lax_issuer_reads(&self, req: &GrantRequest<'_>) -> Repository {
        let mut repository = req.repository.clone();
        let Some(Kind::Federated(declared)) =
            repository.push.as_mut().and_then(|p| p.kind.as_mut())
        else {
            return repository;
        };
        let job = &self.job;
        let repeated = self.is(Flaw::RepeatOfRefusedIssues)
            && self.refused_request.as_ref().is_some_and(|(r, p, e, c)| {
                r == req.repository
                    && *p == req.phase
                    && *e == req.exceptional
                    && c.as_slice() == req.coordinates
            });
        let elsewhere = self.is(Flaw::ClaimsCheckedForTheFirstRepositoryOnly)
            && self
                .first_claims
                .as_ref()
                .is_some_and(|first| first != req.repository);
        let skipped = (self.is(Flaw::SkipsClaimsOutsideStage) && req.phase != GrantPhase::Stage)
            || repeated
            || elsewhere;
        if skipped
            || (self.is(Flaw::ClaimsMatchedBySubstring) && job.subject.contains(&declared.subject))
        {
            declared.subject.clone_from(&job.subject);
        }
        if skipped || (self.is(Flaw::ExceptionalIgnoresEnvironment) && req.exceptional) {
            declared
                .exceptional_environment
                .clone_from(&job.environment);
            declared.environment.clone_from(&job.environment);
        }
        if skipped {
            declared.exceptional_subject.clone_from(&job.subject);
        }
        if self.is(Flaw::IgnoresPinnedClaims) {
            declared.pinned_claims.clone_from(&job.pinned);
        }
        if skipped || (self.is(Flaw::ExceptionalIgnoresAudienceAndPinned) && req.exceptional) {
            declared.audience.clone_from(&job.audience);
            declared.pinned_claims.clone_from(&job.pinned);
        }
        if self.is(Flaw::NoPinnedClaimIsIssued) && job.pinned.is_empty() {
            declared.pinned_claims.clear();
        }
        if self.is(Flaw::ExceptionalFallsBackWhenUndeclared)
            && req.exceptional
            && declared.exceptional_subject.is_empty()
            && declared.exceptional_environment.is_empty()
        {
            declared.exceptional_subject.clone_from(&declared.subject);
            declared
                .exceptional_environment
                .clone_from(&declared.environment);
        }
        if self.is(Flaw::PinnedOnlyFirstCompared)
            && let Some((key, value)) = declared.pinned_claims.iter().next()
            && job.pinned.get(key) == Some(value)
        {
            declared.pinned_claims.clone_from(&job.pinned);
        }
        if self.is(Flaw::PinnedSubset)
            && declared
                .pinned_claims
                .iter()
                .all(|(key, value)| job.pinned.get(key) == Some(value))
        {
            declared.pinned_claims.clone_from(&job.pinned);
        }
        if self.is(Flaw::ClaimsMatchedIgnoringCase) {
            let (subject, environment) = if req.exceptional {
                (
                    &mut declared.exceptional_subject,
                    &mut declared.exceptional_environment,
                )
            } else {
                (&mut declared.subject, &mut declared.environment)
            };
            if subject.eq_ignore_ascii_case(&job.subject) {
                subject.clone_from(&job.subject);
            }
            if environment.eq_ignore_ascii_case(&job.environment) {
                environment.clone_from(&job.environment);
            }
            if declared.audience.eq_ignore_ascii_case(&job.audience) {
                declared.audience.clone_from(&job.audience);
            }
            let alike = declared.pinned_claims.len() == job.pinned.len()
                && declared.pinned_claims.iter().all(|(key, value)| {
                    job.pinned.iter().any(|(job_key, job_value)| {
                        job_key.eq_ignore_ascii_case(key) && job_value.eq_ignore_ascii_case(value)
                    })
                });
            if alike {
                declared.pinned_claims.clone_from(&job.pinned);
            }
        }
        if self.is(Flaw::ExceptionalAcceptsTheOrdinarySubject)
            && req.exceptional
            && job.subject == declared.subject
        {
            declared.exceptional_subject.clone_from(&job.subject);
        }
        if self.is(Flaw::OrdinaryAcceptsTheExceptionalSubject)
            && !req.exceptional
            && job.subject == declared.exceptional_subject
        {
            declared.subject.clone_from(&job.subject);
        }
        if self.is(Flaw::OrdinaryAcceptsTheExceptionalEnvironment)
            && !req.exceptional
            && job.environment == declared.exceptional_environment
        {
            declared.environment.clone_from(&job.environment);
        }
        let (subject, environment) = if req.exceptional {
            (
                &mut declared.exceptional_subject,
                &mut declared.exceptional_environment,
            )
        } else {
            (&mut declared.subject, &mut declared.environment)
        };
        if self.admits(Field::Subject, subject, &job.subject) {
            subject.clone_from(&job.subject);
        }
        if self.admits(Field::Environment, environment, &job.environment) {
            environment.clone_from(&job.environment);
        }
        if self.admits(Field::Audience, &declared.audience, &job.audience) {
            declared.audience.clone_from(&job.audience);
        }
        let alike = declared.pinned_claims.len() == job.pinned.len()
            && declared.pinned_claims.iter().all(|(key, value)| {
                job.pinned.iter().any(|(job_key, job_value)| {
                    self.admits(Field::PinnedName, key, job_key)
                        && self.admits(Field::PinnedValue, value, job_value)
                })
            });
        if alike {
            declared.pinned_claims.clone_from(&job.pinned);
        }
        repository
    }

    /// Whether this stub takes the `claimed` text of a job for the `declared` one.
    fn admits(&self, field: Field, declared: &str, claimed: &str) -> bool {
        declared == claimed
            || match self.flaw {
                Some(Flaw::SubjectMatchedBySuffix) => {
                    field == Field::Subject && claimed.ends_with(declared)
                }
                Some(Flaw::AudienceMatchedByContainment) => {
                    field == Field::Audience && claimed.contains(declared)
                }
                Some(Flaw::EnvironmentMatchedByPrefix) => {
                    field == Field::Environment && claimed.starts_with(declared)
                }
                Some(Flaw::PinnedValueMatchedByContainment) => {
                    field == Field::PinnedValue && claimed.contains(declared)
                }
                Some(Flaw::ClaimsMatchedAfterFolding) => folded(claimed) == folded(declared),
                Some(Flaw::ClaimsMatchedAfterTrimming) => claimed.trim() == declared.trim(),
                Some(Flaw::ClaimsMatchedWithoutInvisibleCharacters) => {
                    without_invisible(claimed) == without_invisible(declared)
                }
                Some(Flaw::ClaimsMatchedUpToANul) => up_to_nul(claimed) == up_to_nul(declared),
                _ => false,
            }
    }
}

impl GrantSource for Flawed {
    fn grant(&mut self, req: &GrantRequest<'_>) -> Result<Grant, SeamError> {
        if self.is(Flaw::ReissuesTheExpiredGrant)
            && let Some((secret, expires)) = &self.cached
            && *expires <= self.clock.now_millis()
        {
            return Ok(Grant::issued(secret.clone(), *expires));
        }
        if self.unreachable_seen {
            self.unreachable_seen = false;
            return Err(SeamError::GrantRefused(GrantFault::Used));
        }
        if self.is(Flaw::LimitIsCheckedForStageOnly)
            && req.phase == GrantPhase::Stage
            && req.coordinates.len() > self.limit
        {
            return Err(SeamError::Refused(Reason::Credential));
        }
        let mut phase = req.phase;
        if self.is(Flaw::ScopeStickyToFirstRequest) {
            phase = *self.first_phase.get_or_insert(req.phase);
        }
        if self.is(Flaw::ReadIsIssuedAsStage) && phase == GrantPhase::Read {
            phase = GrantPhase::Stage;
        }
        if self.is(Flaw::CumulativeLimit)
            && self.covered.saturating_add(req.coordinates.len()) > self.limit
        {
            return Err(SeamError::Refused(Reason::Credential));
        }
        let anything = [coordinate("*")];
        let normalized: Vec<Coordinate>;
        let mut coordinates =
            if self.is(Flaw::EmptyRequestCoversAnything) && req.coordinates.is_empty() {
                &anything[..]
            } else if self.is(Flaw::NormalizesCoordinateFields) {
                normalized = req
                    .coordinates
                    .iter()
                    .map(|c| coordinate(&c.name))
                    .collect();
                normalized.as_slice()
            } else {
                req.coordinates
            };
        if self.is(Flaw::EqualScopesCommitDenied)
            && phase == GrantPhase::Commit
            && let Some(Kind::Federated(declared)) =
                req.repository.push.as_ref().and_then(|p| p.kind.as_ref())
            && declared.stage_scope == declared.commit_scope
        {
            return Err(SeamError::GrantRefused(GrantFault::Denied));
        }
        if self.is(Flaw::CommitGrantDropsACoordinate)
            && phase == GrantPhase::Commit
            && coordinates.len() == 2
        {
            coordinates = &coordinates[..1];
        }
        if self.is(Flaw::DropsTheLastCoordinate)
            && phase == GrantPhase::Stage
            && coordinates.len() == 2
        {
            coordinates = &coordinates[..1];
        }
        if self.is(Flaw::AllowsOneOverTheLimit)
            && Some(coordinates.len()) == self.limit.checked_add(1)
        {
            coordinates = &coordinates[..self.limit];
        }
        let declared_repository;
        let asked = if self.is(Flaw::StickyRepository) {
            declared_repository = self
                .first_repository
                .get_or_insert_with(|| req.repository.clone())
                .clone();
            GrantRequest {
                repository: &declared_repository,
                ..*req
            }
        } else {
            GrantRequest { ..*req }
        };
        if self.is(Flaw::ClaimsCheckedForTheFirstRepositoryOnly) {
            self.first_claims
                .get_or_insert_with(|| req.repository.clone());
        }
        let repository = self.as_a_lax_issuer_reads(&asked);
        let exchanged = self.inner.exchanges().len();
        let exceptional =
            (req.exceptional || self.sticky_exceptional) && !self.is(Flaw::IgnoresExceptional);
        let result = self.inner.grant(&GrantRequest {
            repository: &repository,
            phase,
            coordinates,
            exceptional,
        });
        if self.is(Flaw::StickyExceptional) && req.exceptional && result.is_ok() {
            self.sticky_exceptional = true;
        }
        let (granted_before, refused_before) = (self.issued, self.refused);
        if result.is_ok() {
            self.covered += coordinates.len();
            self.issued += 1;
        } else {
            self.refused = true;
        }
        if self.is(Flaw::RepeatOfRefusedIssues)
            && matches!(result, Err(SeamError::GrantRefused(GrantFault::Denied)))
        {
            self.refused_request = Some((
                req.repository.clone(),
                req.phase,
                req.exceptional,
                req.coordinates.to_vec(),
            ));
        }
        match (self.flaw, result) {
            (Some(Flaw::AnonymousStageOrReadGetsAGrant), Ok(grant))
                if grant.is_none() && phase != GrantPhase::Commit =>
            {
                Ok(Grant::issued(b"x".to_vec(), 1))
            }
            (Some(Flaw::NonStageGrantsNeverExpire), Ok(grant))
                if phase != GrantPhase::Stage && !grant.is_none() =>
            {
                Ok(Grant::issued(grant.secret().to_vec(), u64::MAX))
            }
            (Some(Flaw::ExpiryRoundedToSecond), Ok(grant)) if !grant.is_none() => {
                Ok(Grant::issued(
                    grant.secret().to_vec(),
                    grant.expires_ms().unwrap_or_default() / 1000 * 1000,
                ))
            }
            (Some(Flaw::AnonymousGetsAGrant), Ok(grant)) if grant.is_none() => {
                Ok(Grant::issued(b"x".to_vec(), 1))
            }
            (Some(Flaw::CommitGrantHasTheStageScope), Ok(grant))
                if phase == GrantPhase::Commit && !grant.is_none() =>
            {
                let mut all = texts(grant.secret());
                if let Some(scope) = all.first_mut() {
                    *scope = STAGE_SCOPE.to_owned();
                }
                Ok(Grant::issued(
                    framed(&all),
                    grant.expires_ms().unwrap_or_default(),
                ))
            }
            (Some(Flaw::IgnoresTheDeclaredScopesAndLocation), Ok(grant)) if !grant.is_none() => {
                let scope = match phase {
                    GrantPhase::Stage => STAGE_SCOPE,
                    GrantPhase::Commit => COMMIT_SCOPE,
                    GrantPhase::Read => READ_SCOPE,
                };
                let mut all = texts(grant.secret());
                for (text, own) in all.iter_mut().zip([scope, LOCATION]) {
                    own.clone_into(text);
                }
                Ok(Grant::issued(
                    framed(&all),
                    grant.expires_ms().unwrap_or_default(),
                ))
            }
            (Some(Flaw::UsedIsDenied), Err(SeamError::GrantRefused(GrantFault::Used))) => {
                Err(SeamError::GrantRefused(GrantFault::Denied))
            }
            (
                Some(Flaw::CommitWithoutScopeIsIssued),
                Err(SeamError::GrantRefused(GrantFault::Denied)),
            ) if req.phase == GrantPhase::Commit && self.inner.exchanges().len() > exchanged => {
                // The exchange happened and the issuer refused it for the scope; a refusal
                // before any exchange is one of the claims.
                Ok(Grant::issued(b"commit".to_vec(), 1))
            }
            (
                Some(Flaw::UnreachableSpendsTheToken),
                Err(SeamError::GrantRefused(GrantFault::Unreachable)),
            ) => {
                self.unreachable_seen = true;
                Err(SeamError::GrantRefused(GrantFault::Unreachable))
            }
            (Some(Flaw::CorrectsTheSkew), Ok(grant)) if self.offset_ms != 0 => Ok(Grant::issued(
                grant.secret().to_vec(),
                grant
                    .expires_ms()
                    .unwrap_or_default()
                    .saturating_add_signed(-self.offset_ms),
            )),
            (Some(Flaw::LogsTheSecret), Ok(grant)) => {
                self.log.extend_from_slice(grant.secret());
                Ok(grant)
            }
            (Some(Flaw::LogsTheSecretReversed), Ok(grant)) => {
                self.log.extend(grant.secret().iter().rev());
                Ok(grant)
            }
            (Some(Flaw::LogsTheMiddleOfTheSecret), Ok(grant)) => {
                let secret = grant.secret();
                self.log.extend_from_slice(
                    &secret[2.min(secret.len())..secret.len().saturating_sub(2)],
                );
                Ok(grant)
            }
            (Some(Flaw::LogsTheSecretInHex), Ok(grant)) => {
                let hex: String = grant.secret().iter().map(|b| format!("{b:02x}")).collect();
                self.log.extend_from_slice(hex.as_bytes());
                Ok(grant)
            }
            (Some(Flaw::LogsTheCommitSecret), Ok(grant)) if phase == GrantPhase::Commit => {
                self.log.extend_from_slice(grant.secret());
                Ok(grant)
            }
            (Some(Flaw::LogsTheExceptionalSecret), Ok(grant)) if req.exceptional => {
                self.log.extend_from_slice(grant.secret());
                Ok(grant)
            }
            (Some(Flaw::LogsTheSecretInBase64), Ok(grant)) => {
                self.log
                    .extend_from_slice(base64(grant.secret()).as_bytes());
                Ok(grant)
            }
            (Some(Flaw::LogsTheSecretSpaced), Ok(grant)) => {
                for byte in grant.secret() {
                    self.log.extend_from_slice(&[*byte, b' ']);
                }
                Ok(grant)
            }
            (Some(Flaw::LogsThePrefixOfTheSecret), Ok(grant)) => {
                let secret = grant.secret();
                self.log.extend_from_slice(&secret[..secret.len() * 2 / 5]);
                Ok(grant)
            }
            (Some(Flaw::LogsTheSecretFromTheSecondGrant), Ok(grant)) if granted_before > 0 => {
                self.log.extend_from_slice(grant.secret());
                Ok(grant)
            }
            (Some(Flaw::LogsTheSecretAfterARefusal), Ok(grant)) if refused_before => {
                self.log.extend_from_slice(grant.secret());
                Ok(grant)
            }
            (Some(Flaw::ReissuesTheExpiredGrant), Ok(grant)) => {
                self.cached = Some((
                    grant.secret().to_vec(),
                    grant.expires_ms().unwrap_or_default(),
                ));
                Ok(grant)
            }
            (_, result) => result,
        }
    }
}
