// SPDX-License-Identifier: Apache-2.0
//! The CI identity token: where it comes from, what it says, and the checks made of what it says
//! before it is spent.
//!
//! An [`IdentityToken`] is a signed JWT that the issuer accepts once. It is neither `Debug` nor
//! `Display` nor `Clone`, and it is overwritten when dropped. This crate reads its claims (the
//! issuer verifies the signature, not this crate) so that ucdk is stricter than the issuer and
//! never looser: [`check_identity`] refuses a token for another issuer, audience, subject or
//! environment, one that differs from a pinned claim, one that is expired or not yet valid, one
//! with no `jti`, and one whose source repository is not public, before any exchange.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Map, Value};
use ucdk_ledger::MAX_CLOCK_SKEW;

use crate::transport::{Header, Method, Request, Transport};
use crate::url::Url;

/// The longest identity token mopm reads.
const MAX_JWT_BYTES: usize = 8192;

/// The identity token could not be had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityUnavailable(pub &'static str);

impl fmt::Display for IdentityUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "no identity token: {}", self.0)
    }
}

impl std::error::Error for IdentityUnavailable {}

/// Gives the job a new identity token for an audience. Every call returns a new token.
pub trait IdentitySource {
    fn identity_token(&mut self, audience: &str) -> Result<IdentityToken, IdentityUnavailable>;
}

/// A signed identity token. The bytes are overwritten when it is dropped.
pub struct IdentityToken {
    jwt: Vec<u8>,
}

impl Drop for IdentityToken {
    fn drop(&mut self) {
        self.jwt.fill(0);
    }
}

fn base64url(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for byte in text {
        let sextet = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(sextet);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            // The accumulator holds fewer than `bits + 8` bits here, so this is one byte.
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    // One character alone holds no byte, and the unused bits of the last must be zero, so one
    // text has one encoding.
    (bits < 6 && acc == 0).then_some(out)
}

impl IdentityToken {
    /// A token of the form `header.payload.signature`, each part base64url, at most 8192
    /// bytes: a text that cannot be, is refused here, and so never reaches a request body.
    pub fn new(jwt: Vec<u8>) -> Result<Self, IdentityUnavailable> {
        let alphabet = jwt
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        let parts = jwt.split(|b| *b == b'.').count();
        if !alphabet || parts != 3 || jwt.len() > MAX_JWT_BYTES {
            return Err(IdentityUnavailable("the token is not a JWT"));
        }
        Ok(Self { jwt })
    }

    /// The bytes that go in the exchange request. Only that request reads them.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.jwt
    }

    /// The claims of the payload. The signature is not checked here.
    pub fn claims(&self) -> Result<Claims, IdentityUnavailable> {
        let payload = self.jwt.split(|b| *b == b'.').nth(1).unwrap_or_default();
        let bytes =
            base64url(payload).ok_or(IdentityUnavailable("the payload is not base64url"))?;
        match serde_json::from_slice(&bytes) {
            Ok(Value::Object(map)) => Ok(Claims(map)),
            _ => Err(IdentityUnavailable("the payload is not a JSON object")),
        }
    }
}

/// The claims of an identity token's payload.
pub struct Claims(Map<String, Value>);

impl Claims {
    pub fn text(&self, name: &str) -> Option<&str> {
        self.0.get(name)?.as_str()
    }

    /// A numeric date in whole seconds; a claim that is not a non-negative integer is none.
    pub fn seconds(&self, name: &str) -> Option<u64> {
        self.0.get(name)?.as_u64()
    }

    /// The audiences: the one text or the list of texts; none for a claim of any other type.
    fn audiences(&self) -> Option<Vec<&str>> {
        match self.0.get("aud")? {
            Value::String(one) => Some(vec![one.as_str()]),
            Value::Array(list) => list.iter().map(Value::as_str).collect(),
            _ => None,
        }
    }
}

/// What a channel's declaration says the token must be.
pub struct Expected<'a> {
    pub issuer: &'a str,
    pub audience: &'a str,
    pub subject: &'a str,
    pub environment: &'a str,
    pub pinned: &'a BTreeMap<String, String>,
}

/// The first claim that differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mismatch {
    Issuer,
    /// Not exactly the declared audience: another, or a list that holds more than it.
    Audience,
    Subject,
    Environment,
    /// A pinned claim, by name.
    Pinned(String),
    /// `repository_visibility` is not `public`.
    Visibility,
    /// No `exp`, or it is not after now.
    Expired,
    /// `nbf` is later than now plus the clock-skew allowance.
    NotYetValid,
    /// No `iat`, or it is later than now plus the clock-skew allowance.
    IssuedAt,
    /// No `jti`, which is what the issuer accepts once.
    TokenId,
    /// No `repository_id`, which the actor is made from.
    Repository,
}

/// What a token that passed says, which the exchange needs.
pub struct Verified {
    pub jti: String,
    pub repository_id: String,
}

fn non_empty<'a>(claims: &'a Claims, name: &str) -> Option<&'a str> {
    claims.text(name).filter(|t| !t.is_empty())
}

/// Compares `claims` with what the declaration says, at `now_ms`. Stricter than the issuer: the
/// audience must be exactly the declared one, an expiry is never excused by a leeway, and the
/// source repository must be public. A mismatch drops the token unspent.
pub fn check_identity(
    claims: &Claims,
    expected: &Expected<'_>,
    now_ms: u64,
) -> Result<Verified, Mismatch> {
    if claims.text("iss") != Some(expected.issuer) {
        return Err(Mismatch::Issuer);
    }
    if claims.audiences() != Some(vec![expected.audience]) {
        return Err(Mismatch::Audience);
    }
    if claims.text("sub") != Some(expected.subject) {
        return Err(Mismatch::Subject);
    }
    if claims.text("environment").unwrap_or_default() != expected.environment {
        return Err(Mismatch::Environment);
    }
    for (name, value) in expected.pinned {
        if claims.text(name) != Some(value.as_str()) {
            return Err(Mismatch::Pinned(name.clone()));
        }
    }
    if claims.text("repository_visibility") != Some("public") {
        return Err(Mismatch::Visibility);
    }
    let skew = u64::try_from(MAX_CLOCK_SKEW.as_millis()).unwrap_or(u64::MAX);
    let seconds = |name| claims.seconds(name).map(|s| s.saturating_mul(1000));
    if seconds("exp").is_none_or(|exp| exp <= now_ms) {
        return Err(Mismatch::Expired);
    }
    if claims.0.contains_key("nbf")
        && seconds("nbf").is_none_or(|nbf| nbf > now_ms.saturating_add(skew))
    {
        return Err(Mismatch::NotYetValid);
    }
    if seconds("iat").is_none_or(|iat| iat > now_ms.saturating_add(skew)) {
        return Err(Mismatch::IssuedAt);
    }
    let jti = non_empty(claims, "jti").ok_or(Mismatch::TokenId)?;
    let repository_id = non_empty(claims, "repository_id").ok_or(Mismatch::Repository)?;
    Ok(Verified {
        jti: jti.to_owned(),
        repository_id: repository_id.to_owned(),
    })
}

/// The identity-token endpoint of GitHub Actions: a `GET` of the request URL the job is given,
/// with `audience` added, carrying the request token the job is given as a bearer. The caller
/// reads both from the job and passes them in; this crate reads no environment variable.
pub struct GithubIdentity<T: Transport> {
    transport: T,
    url: Url,
    request_token: Vec<u8>,
}

impl<T: Transport> Drop for GithubIdentity<T> {
    fn drop(&mut self) {
        self.request_token.fill(0);
    }
}

impl<T: Transport> GithubIdentity<T> {
    /// `None` for a request token that is not header-safe.
    pub fn new(transport: T, url: Url, request_token: Vec<u8>) -> Option<Self> {
        let usable = Header::bearer(&request_token).is_some();
        usable.then_some(Self {
            transport,
            url,
            request_token,
        })
    }
}

impl<T: Transport> IdentitySource for GithubIdentity<T> {
    fn identity_token(&mut self, audience: &str) -> Result<IdentityToken, IdentityUnavailable> {
        let url = self.url.with_param("audience", audience);
        let headers = Header::bearer(&self.request_token)
            .into_iter()
            .chain([Header::plain("accept", "application/json")])
            .collect();
        let request = Request {
            method: Method::Get,
            url,
            headers,
            body: &[],
        };
        let mut response = self
            .transport
            .send(&request)
            .map_err(|_| IdentityUnavailable("the platform could not be reached"))?;
        let value = if response.is_success() {
            serde_json::from_slice::<Value>(&response.body).ok()
        } else {
            None
        };
        response.body.fill(0);
        let Some(Value::Object(mut map)) = value else {
            return Err(IdentityUnavailable("the platform gave no token"));
        };
        match map.remove("value") {
            Some(Value::String(jwt)) => IdentityToken::new(jwt.into_bytes()),
            _ => Err(IdentityUnavailable("the platform gave no token")),
        }
    }
}
