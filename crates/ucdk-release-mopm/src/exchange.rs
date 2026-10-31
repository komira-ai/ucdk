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

use crate::config::{Config, MOPM_TOKEN_TTL};
use crate::errors::{error_code, rate_limited};
use crate::grants::Envelope;
use crate::identity::{Expected, IdentitySource, IdentityToken, check_identity};
use crate::mapping::check_shape;
use crate::transport::{Header, Method, Request, Response, Transport};

/// The most coordinates one grant covers: never over mopm's 100 distinct package names.
pub const GRANT_LIMIT: usize = 100;

/// The issuer of the identity token of a GitHub Actions job.
pub const GITHUB_ISSUER: &str = "https://token.actions.githubusercontent.com";

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
    /// The channel, the registry the exchange is sent to, and the `Retry-After` ceiling.
    pub config: Config,
    /// The location of the one repository this source serves, compared byte for byte.
    pub location: String,
    /// The issuer the identity token must name: [`GITHUB_ISSUER`] on GitHub Actions.
    pub issuer: String,
    pub actor: ActorForm,
}

struct Wiped(Vec<u8>);

impl Drop for Wiped {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

struct Minted {
    token: Wiped,
    expires_ms: u64,
}

fn refused(fault: GrantFault) -> SeamError {
    SeamError::GrantRefused(fault)
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// The year of the Unix epoch: no expiry is earlier.
const EPOCH_YEAR: u64 = 1970;

fn days_in_month(year: u64, month: u64) -> u64 {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from the Unix epoch to the civil date, for a valid date in or after its year.
fn days_from_civil(year: u64, month: u64, day: u64) -> u64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe).saturating_sub(719_468)
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
    let valid = year >= EPOCH_YEAR
        && (1..=12).contains(&month)
        && (1..=days_in_month(year, month)).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60;
    valid.then(|| {
        let days = days_from_civil(year, month, day);
        (((days * 24 + hour) * 60 + minute) * 60 + second) * 1000 + fraction
    })
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

fn echoed(value: &Value) -> Option<BTreeSet<(String, String)>> {
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

fn interpret(
    response: &Response,
    asked: &BTreeSet<(String, String)>,
    now_ms: u64,
    ceiling: Duration,
) -> Result<Minted, SeamError> {
    if !response.is_success() {
        return Err(exchange_failure(response, ceiling));
    }
    let unreadable = || refused(GrantFault::Unreachable);
    let Ok(Value::Object(mut map)) = serde_json::from_slice(&response.body) else {
        return Err(unreadable());
    };
    let Some(Value::String(token)) = map.remove("token") else {
        return Err(unreadable());
    };
    let token = Wiped(token.into_bytes());
    let visible = !token.0.is_empty() && token.0.iter().all(u8::is_ascii_graphic);
    let stated = map
        .get("expires_at")
        .and_then(Value::as_str)
        .and_then(utc_millis);
    let packages = map.get("packages").and_then(echoed);
    let (true, Some(stated), Some(packages)) = (visible, stated, packages) else {
        return Err(unreadable());
    };
    let plain = map.get("token_kind").and_then(Value::as_str) == Some("publish");
    if !plain || packages != *asked {
        return Err(refused(GrantFault::Denied));
    }
    if stated <= now_ms {
        return Err(refused(GrantFault::Expired));
    }
    // The expiry is on the issuer's clock. A token never outlives its stated life on ours, and
    // an issuer whose clock is ahead cannot make it last longer than the life mopm promises.
    let cap = now_ms.saturating_add(millis(MOPM_TOKEN_TTL));
    Ok(Minted {
        token,
        expires_ms: stated.min(cap),
    })
}

/// Reads the answer to an exchange and overwrites its body, which holds the minted token.
fn read_exchange(
    response: &mut Response,
    asked: &BTreeSet<(String, String)>,
    now_ms: u64,
    ceiling: Duration,
) -> Result<Minted, SeamError> {
    let result = interpret(response, asked, now_ms, ceiling);
    response.body.fill(0);
    result
}

/// The request body of an exchange: the identity token and the `(name, version)` pairs. The
/// token is checked to be base64url and dots, so it needs no escaping.
fn exchange_body(jwt: &[u8], asked: &BTreeSet<(String, String)>) -> Wiped {
    let packages: Vec<Value> = asked
        .iter()
        .map(|(name, version)| serde_json::json!({ "name": name, "version": version }))
        .collect();
    let mut body = Vec::with_capacity(jwt.len() + 64);
    body.extend_from_slice(br#"{"jwt":""#);
    body.extend_from_slice(jwt);
    body.extend_from_slice(br#"","packages":"#);
    body.extend_from_slice(Value::Array(packages).to_string().as_bytes());
    body.push(b'}');
    Wiped(body)
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
            body: &body.0,
        };
        let mut response = self
            .transport
            .send(&request)
            .map_err(|_| refused(GrantFault::Unreachable))?;
        let ceiling = self.setup.config.retry_after_ceiling;
        read_exchange(&mut response, asked, self.clock.now_millis(), ceiling)
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
        if !public || req.repository.location != self.setup.location {
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
            std::mem::take(&mut minted.token.0),
        );
        Ok(Grant::issued(envelope.encode(), minted.expires_ms))
    }
}
