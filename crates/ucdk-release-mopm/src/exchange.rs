// SPDX-License-Identifier: Apache-2.0
//! The mopm grant source: it exchanges the job's CI identity token for a short-lived mopm
//! token and wraps that token in the envelope of [`crate::grants`].
//!
//! Nothing standing is held. The only credential is the identity token the CI platform gives
//! the job for one exchange; it is asked for once per grant, used in one request body, never
//! sent again (a lost answer burns it, because the issuer accepts each token once), and
//! overwritten when it is dropped. The token mopm answers with lives only in the grant,
//! which overwrites it on drop. Neither is written to disk, an argument vector, an error, a
//! `Debug` or a `Display`.
//!
//! Before it asks for an identity token it checks everything it can without one: the
//! repository is public and the one it serves, the coordinates are within the limit and
//! pass the grammar and set checks (on the beta configured form the exchange creates a missing
//! package, so a misspelled name must never reach it). Then it compares the token's issuer,
//! audience, subject, environment, pinned claims, visibility and times with the declaration.
//! A Read grant is never exchanged: it is [`Grant::none`].

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;
use ucdk_ledger::Clock;
use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{Coordinate, Reason, Visibility};
use ucdk_release_seams::{Grant, GrantRequest, GrantSource, SeamError};
use zeroize::{Zeroize, Zeroizing};

use crate::config::{Config, MOPM_TOKEN_TTL};
use crate::errors::{error_code, rate_limited};
use crate::grants::Envelope;
use crate::identity::{Expected, IdentitySource, IdentityToken, check_identity};
use crate::json::read_object;
use crate::mapping::check_shape;
use crate::transport::{Header, Method, Request, Response, Transport};

/// The most coordinates one grant covers: never over mopm's 100 distinct package names.
pub const GRANT_LIMIT: usize = 100;

/// Whose actor the group id is made from. Where the grant source learns it is the caller's
/// decision, since the exchange answer does not carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorForm {
    /// The configured CI identity: `ci:gh-repo:<repository id>`, the id read from the token.
    ConfiguredCi,
    /// The account the identity acts for: `u:<GitHub id>`.
    Account(u64),
}

/// What the grant source is told.
pub struct GrantSourceConfig {
    /// The channel, the registry the exchange is sent to, and the `Retry-After` ceiling. The
    /// registry is also the one repository this source serves: a repository whose location is
    /// not the registry's, byte for byte, is refused, so a token is never exchanged at one
    /// registry for a grant labelled with another.
    pub config: Config,
    /// The issuer the identity token must name (its `iss` claim), compared exactly. The caller
    /// states it: nothing here assumes a CI platform.
    pub issuer: String,
    pub actor: ActorForm,
}

struct Minted {
    token: Zeroizing<Vec<u8>>,
    expires_ms: u64,
}

fn refused(fault: GrantFault) -> SeamError {
    SeamError::GrantRefused(fault)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Days from the start of the proleptic Gregorian calendar's year zero to the Unix epoch.
const DAYS_TO_EPOCH: u64 = 719_468;

fn days_in_month(year: u64, month: u64) -> u64 {
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from the Unix epoch to a valid civil date; `None` for a date before the epoch.
fn days_from_civil(year: u64, month: u64, day: u64) -> Option<u64> {
    // The year counted from March, so that a leap day is the last day of its year.
    let y = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe).checked_sub(DAYS_TO_EPOCH)
}

/// The milliseconds since the epoch of an instant written `YYYY-MM-DDTHH:MM:SS[.f...]Z`, the
/// only form mopm writes. Anything else, a date that does not exist and a year before the epoch's
/// included, is none.
pub(crate) fn utc_millis(text: &str) -> Option<u64> {
    let digits = |from: usize, to: usize| -> Option<u64> {
        let part = text.get(from..to)?;
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let separators = [(4, '-'), (7, '-'), (10, 'T'), (13, ':'), (16, ':')];
    if !text.is_ascii()
        || !separators
            .iter()
            .all(|(at, c)| text.get(*at..).is_some_and(|t| t.starts_with(*c)))
    {
        return None;
    }
    let (year, month, day) = (digits(0, 4)?, digits(5, 7)?, digits(8, 10)?);
    let (hour, minute, second) = (digits(11, 13)?, digits(14, 16)?, digits(17, 19)?);
    let fraction = match text.get(19..)? {
        "Z" => 0,
        rest => {
            let inner = rest.strip_prefix('.')?.strip_suffix('Z')?;
            let ok = (1..=9).contains(&inner.len()) && inner.bytes().all(|b| b.is_ascii_digit());
            let first = format!("{inner:0<3}");
            ok.then(|| first[..3].parse::<u64>().ok())??
        }
    };
    let valid = (1..=12).contains(&month)
        && (1..=days_in_month(year, month)).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60;
    let days = valid.then(|| days_from_civil(year, month, day))??;
    Some((((days * 24 + hour) * 60 + minute) * 60 + second) * 1000 + fraction)
}

fn already_used(body: &[u8]) -> bool {
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| {
            Some(
                v.pointer("/error/message")?
                    .as_str()?
                    .contains("token already used"),
            )
        })
        .unwrap_or(false)
}

/// What a failed exchange means. `Used` is read from the one message mopm gives for it, since
/// it has no code of its own; the message is never copied into the error. A server failure or a
/// lost answer may have spent the identity token, so it is `Unreachable`: the caller asks for a
/// new one and never sends this one again.
fn exchange_failure(response: &Response, ceiling: Duration) -> SeamError {
    let code = error_code(&response.body);
    match (response.status, code.as_deref()) {
        (429, _) => rate_limited(response, ceiling),
        (401, Some("oidc_token_invalid")) if already_used(&response.body) => {
            refused(GrantFault::Used)
        }
        (401 | 403, _) => refused(GrantFault::Denied),
        (404, Some("route_not_found")) => SeamError::Refused(Reason::Channel),
        (500..=599, _) => refused(GrantFault::Unreachable),
        (status, _) => SeamError::Permanent(format!("the exchange answered {status}")),
    }
}

/// The `(name, version)` pairs of the answer, in its order and with any repeat.
fn echoed(value: &Value) -> Option<Vec<(String, String)>> {
    value
        .as_array()?
        .iter()
        .map(|entry| {
            let name = entry.get("name")?.as_str()?;
            let version = entry.get("version")?.as_str()?;
            Some((name.to_owned(), version.to_owned()))
        })
        .collect()
}

/// When the exchange was sent and when its answer came back, on the local clock.
#[derive(Clone, Copy)]
struct Times {
    sent_ms: u64,
    received_ms: u64,
}

/// Reads a successful answer. Unknown fields are ignored; a repeated field or a field of the
/// wrong type is `Unreachable` (the answer cannot be read; so is a token that is not
/// header-safe, which the envelope refuses);
/// another kind of token, or packages that are not exactly the ones asked for (a repeat
/// included, in any order), is `Denied`; an expiry not after the answer came back is `Expired`.
fn interpret(
    response: &Response,
    asked: &BTreeSet<(String, String)>,
    times: Times,
    ceiling: Duration,
) -> Result<Minted, SeamError> {
    if !response.is_success() {
        return Err(exchange_failure(response, ceiling));
    }
    let unreadable = || refused(GrantFault::Unreachable);
    let fields = read_object(&response.body, Some("token"))
        .filter(|f| !f.repeated)
        .ok_or_else(unreadable)?;
    let mut text = fields.secret.ok_or_else(unreadable)?;
    // The text's buffer moves into the token, which is wiped on drop; what is left is empty.
    let token = Zeroizing::new(std::mem::take(&mut *text).into_bytes());
    let map = fields.rest;
    let stated = map
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(utc_millis);
    let packages = map.get("packages").and_then(echoed);
    let (Some(stated), Some(packages)) = (stated, packages) else {
        return Err(unreadable());
    };
    let plain = map.get("token_kind").and_then(Value::as_str) == Some("publish");
    let distinct: BTreeSet<(String, String)> = packages.iter().cloned().collect();
    if !plain || distinct.len() != packages.len() || distinct != *asked {
        return Err(refused(GrantFault::Denied));
    }
    // The expiry is on the issuer's clock. A token never outlives its stated life on ours,
    // counted from when the request was sent (the issuer may have minted it at once, and the
    // answer may have been slow to arrive), so an issuer whose clock is ahead cannot make it
    // last longer than the life mopm promises.
    let cap = times.sent_ms.saturating_add(millis(MOPM_TOKEN_TTL));
    let expires_ms = stated.min(cap);
    if expires_ms <= times.received_ms {
        return Err(refused(GrantFault::Expired));
    }
    Ok(Minted { token, expires_ms })
}

/// Reads the answer to an exchange and overwrites its body, which holds the minted token.
fn read_exchange(
    response: &mut Response,
    asked: &BTreeSet<(String, String)>,
    times: Times,
    ceiling: Duration,
) -> Result<Minted, SeamError> {
    let result = interpret(response, asked, times, ceiling);
    response.body.zeroize();
    result
}

/// The request body of an exchange: the identity token and the `(name, version)` pairs. The
/// token is checked to be base64url and dots, so it needs no escaping. The body is sized once,
/// so that no reallocation leaves a copy of the token behind, and it is wiped on drop.
fn exchange_body(jwt: &[u8], asked: &BTreeSet<(String, String)>) -> Zeroizing<Vec<u8>> {
    const OPEN: &[u8] = br#"{"jwt":""#;
    const MIDDLE: &[u8] = br#"","packages":"#;
    let packages: Vec<Value> = asked
        .iter()
        .map(|(name, version)| serde_json::json!({ "name": name, "version": version }))
        .collect();
    let packages = Value::Array(packages).to_string();
    let size = OPEN.len() + jwt.len() + MIDDLE.len() + packages.len() + 1;
    let mut body = Zeroizing::new(Vec::with_capacity(size));
    body.extend_from_slice(OPEN);
    body.extend_from_slice(jwt);
    body.extend_from_slice(MIDDLE);
    body.extend_from_slice(packages.as_bytes());
    body.push(b'}');
    body
}

pub struct MopmGrantSource<T, I, C> {
    setup: GrantSourceConfig,
    transport: T,
    identity: I,
    clock: C,
    /// The `jti` of every identity token sent, so the same one is never sent twice.
    spent: BTreeSet<String>,
}

impl<T: Transport, I: IdentitySource, C: Clock> MopmGrantSource<T, I, C> {
    pub fn new(setup: GrantSourceConfig, transport: T, identity: I, clock: C) -> Self {
        Self {
            setup,
            transport,
            identity,
            clock,
            spent: BTreeSet::new(),
        }
    }

    /// The location of the repository this source serves: the registry's, if it has no query
    /// (a registry with one is a configuration `Config::check` refuses, and serves nothing).
    fn served(&self) -> Option<String> {
        let registry = &self.setup.config.registry;
        (!registry.has_query()).then(|| registry.expose())
    }

    fn exchange(
        &mut self,
        token: &IdentityToken,
        asked: &BTreeSet<(String, String)>,
    ) -> Result<Minted, SeamError> {
        let body = exchange_body(token.bytes(), asked);
        let request = Request {
            method: Method::Post,
            url: self
                .setup
                .config
                .registry
                .join(&["auth", "oidc", "exchange"]),
            headers: vec![
                Header::plain("content-type", "application/json"),
                Header::plain("accept", "application/json"),
            ],
            body: &body,
        };
        let sent_ms = self.clock.now_millis();
        let mut response = self
            .transport
            .send(&request)
            .map_err(|_| refused(GrantFault::Unreachable))?;
        let times = Times {
            sent_ms,
            received_ms: self.clock.now_millis(),
        };
        let ceiling = self.setup.config.retry_after_ceiling;
        read_exchange(&mut response, asked, times, ceiling)
    }
}

impl<T: Transport + Send, I: IdentitySource + Send, C: Clock> GrantSource
    for MopmGrantSource<T, I, C>
{
    fn grant(&mut self, req: &GrantRequest<'_>) -> Result<Grant, SeamError> {
        let Some(Kind::Federated(declared)) =
            req.repository.push.as_ref().and_then(|p| p.kind.as_ref())
        else {
            return Ok(Grant::none());
        };
        if req.phase == GrantPhase::Read {
            return Ok(Grant::none());
        }
        let public = req.repository.visibility == Visibility::Public as i32;
        if !public || Some(&req.repository.location) != self.served().as_ref() {
            return Err(refused(GrantFault::Denied));
        }
        if req.coordinates.len() > GRANT_LIMIT {
            return Err(SeamError::Refused(Reason::Credential));
        }
        if req.coordinates.is_empty() {
            return Err(SeamError::Refused(Reason::Channel));
        }
        let set: Vec<&Coordinate> = req.coordinates.iter().collect();
        let keys = check_shape(&self.setup.config.channel, &set)?;
        let asked: BTreeSet<(String, String)> =
            keys.into_iter().map(|k| (k.name, k.version)).collect();
        // A declaration that names no exceptional subject admits no exceptional run.
        let (subject, environment) = if req.exceptional {
            (
                &declared.exceptional_subject,
                &declared.exceptional_environment,
            )
        } else {
            (&declared.subject, &declared.environment)
        };
        if req.exceptional && subject.is_empty() {
            return Err(refused(GrantFault::Denied));
        }
        let token = self
            .identity
            .identity_token(&declared.audience)
            .map_err(|_| refused(GrantFault::Unreachable))?;
        let claims = token.claims().map_err(|_| refused(GrantFault::Denied))?;
        let expected = Expected {
            issuer: &self.setup.issuer,
            audience: &declared.audience,
            subject,
            environment,
            pinned: &declared.pinned_claims,
        };
        let verified = check_identity(&claims, &expected, self.clock.now_millis())
            .map_err(|_| refused(GrantFault::Denied))?;
        // Recorded before the request goes out: a lost answer burns the token as well.
        if !self.spent.insert(verified.jti) {
            return Err(refused(GrantFault::Used));
        }
        let actor = match self.setup.actor {
            ActorForm::ConfiguredCi => format!("ci:gh-repo:{}", verified.repository_id),
            ActorForm::Account(id) => format!("u:{id}"),
        };
        let mut minted = self.exchange(&token, &asked)?;
        drop(token);
        let envelope = Envelope::new(
            req.phase,
            req.exceptional,
            req.repository.location.clone(),
            actor,
            req.coordinates.to_vec(),
            std::mem::take(&mut *minted.token),
        )
        // A token that is empty or not visible ASCII cannot be sent as a bearer: the answer
        // cannot be used.
        .map_err(|_| refused(GrantFault::Unreachable))?;
        // The encoded buffer moves into the grant, which wipes it on drop; what is left behind
        // is an empty vector.
        let mut secret = envelope.encode();
        Ok(Grant::issued(
            std::mem::take(&mut *secret),
            minted.expires_ms,
        ))
    }
}
