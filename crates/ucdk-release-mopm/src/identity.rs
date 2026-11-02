// SPDX-License-Identifier: Apache-2.0
//! The CI identity token: where it comes from, what it says, and the checks made of what it says
//! before it is spent.
//!
//! An [`IdentityToken`] is a signed JWT that the issuer accepts once. It is neither `Debug` nor
//! `Display` nor `Clone`, and the `zeroize` crate overwrites it with zeros when it is dropped.
//! This crate reads its claims (the issuer verifies the signature, not this crate) so that ucdk
//! is stricter than the issuer and never looser: a payload that names a claim twice has no
//! claims, and [`check_identity`] refuses a token for another issuer, audience, subject or
//! environment, one that differs from a pinned claim, one that is expired or not yet valid, one
//! with no `jti`, and one whose source repository is not public or has no numeric id, before
//! any exchange.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::{Map, Value};
use ucdk_ledger::MAX_CLOCK_SKEW;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::json::read_object;
use crate::transport::{Header, Method, Request, Transport};
use crate::url::Url;

/// The longest identity token mopm reads.
const MAX_JWT_BYTES: usize = 8192;

/// Why no identity token could be had. No variant carries any text of the token or of the
/// platform's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityUnavailable {
    /// The text is not three non-empty base64url parts of at most 8192 bytes in all.
    NotAJwt,
    /// The payload is not one JSON object.
    PayloadNotAnObject,
    /// The payload names a claim more than once.
    RepeatedClaim,
    /// The platform could not be reached.
    Unreachable,
    /// The platform answered, but not with one token.
    NoToken,
}

impl fmt::Display for IdentityUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = match self {
            IdentityUnavailable::NotAJwt => "the token is not a JWT",
            IdentityUnavailable::PayloadNotAnObject => "the payload is not a JSON object",
            IdentityUnavailable::RepeatedClaim => "the payload names a claim twice",
            IdentityUnavailable::Unreachable => "the platform could not be reached",
            IdentityUnavailable::NoToken => "the platform gave no token",
        };
        write!(f, "no identity token: {why}")
    }
}

impl std::error::Error for IdentityUnavailable {}

/// Gives the job a new identity token for an audience. Every call returns a new token.
pub trait IdentitySource {
    fn identity_token(&mut self, audience: &str) -> Result<IdentityToken, IdentityUnavailable>;
}

/// A signed identity token. The bytes, and the decoded payload, are overwritten with zeros when
/// it is dropped.
pub struct IdentityToken {
    jwt: Zeroizing<Vec<u8>>,
    payload: Zeroizing<Vec<u8>>,
}

// The bytes are held in `Zeroizing`, which wipes them on drop, so the token is wiped on drop too.
// This marker is a promise the compiler does not check: every field of `IdentityToken` that holds
// a secret must be `Zeroizing`, or be wiped by a `Drop` of its own, before it is added.
impl ZeroizeOnDrop for IdentityToken {}

/// The bytes of a base64url text without padding, in a buffer that is wiped on drop and sized
/// once. `None` for a text that is not one.
fn base64url(text: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    let mut out = Zeroizing::new(Vec::with_capacity(text.len() * 3 / 4));
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
    /// A token of the form `header.payload.signature`, at most 8192 bytes, each part a
    /// non-empty base64url text without padding whose unused bits are zero (so one token has one
    /// spelling): a text that is not, is refused here, and so never reaches a request body. The
    /// bytes are wiped on drop whether the text is accepted or refused.
    pub fn new(jwt: Vec<u8>) -> Result<Self, IdentityUnavailable> {
        let jwt = Zeroizing::new(jwt);
        if jwt.len() > MAX_JWT_BYTES {
            return Err(IdentityUnavailable::NotAJwt);
        }
        let mut parts = jwt
            .split(|b| *b == b'.')
            .map(|part| base64url(part).filter(|bytes| !bytes.is_empty()));
        let (Some(Some(_header)), Some(Some(payload)), Some(Some(_signature)), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(IdentityUnavailable::NotAJwt);
        };
        drop(parts);
        Ok(Self { jwt, payload })
    }

    /// The bytes that go in the exchange request. Only that request reads them.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.jwt
    }

    /// The claims of the payload. The signature is not checked here. A payload that names a
    /// claim twice has none: another reader could take the other value.
    pub fn claims(&self) -> Result<Claims, IdentityUnavailable> {
        let fields =
            read_object(&self.payload, None).ok_or(IdentityUnavailable::PayloadNotAnObject)?;
        if fields.repeated {
            return Err(IdentityUnavailable::RepeatedClaim);
        }
        Ok(Claims(fields.rest))
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

    /// The audiences: the one text or the list of texts, in order and with any repeat; none
    /// for a claim of any other type.
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
    /// `environment` differs from the declared one, or is present and not text (an absent claim
    /// matches only a declaration that names no environment).
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
    /// No `repository_id` of decimal digits with no leading zero, which the actor is made from.
    Repository,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let claim = match self {
            Mismatch::Issuer => "iss",
            Mismatch::Audience => "aud",
            Mismatch::Subject => "sub",
            Mismatch::Environment => "environment",
            // The name is the declaration's, not the token's.
            Mismatch::Pinned(name) => name,
            Mismatch::Visibility => "repository_visibility",
            Mismatch::Expired => "exp",
            Mismatch::NotYetValid => "nbf",
            Mismatch::IssuedAt => "iat",
            Mismatch::TokenId => "jti",
            Mismatch::Repository => "repository_id",
        };
        write!(
            f,
            "the identity token's {claim} claim is not what the declaration admits"
        )
    }
}

impl std::error::Error for Mismatch {}

/// What a token that passed says, which the exchange needs.
pub struct Verified {
    pub jti: String,
    pub repository_id: String,
}

fn non_empty<'a>(claims: &'a Claims, name: &str) -> Option<&'a str> {
    claims.text(name).filter(|t| !t.is_empty())
}

/// Decimal digits with no leading zero: the form of a repository id, and nothing that could
/// change the shape of the actor made from it.
fn numeric_id(text: &str) -> bool {
    !text.is_empty() && !text.starts_with('0') && text.bytes().all(|b| b.is_ascii_digit())
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
    let environment = match claims.0.get("environment") {
        None => Some(""),
        Some(value) => value.as_str(),
    };
    if environment != Some(expected.environment) {
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
    let repository_id = claims
        .text("repository_id")
        .filter(|id| numeric_id(id))
        .ok_or(Mismatch::Repository)?;
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
    request_token: Zeroizing<Vec<u8>>,
}

// The request token is held in `Zeroizing`, which wipes it on drop; the same promise as on
// `IdentityToken` holds for every field added here.
impl<T: Transport> ZeroizeOnDrop for GithubIdentity<T> {}

impl<T: Transport> GithubIdentity<T> {
    /// `None` for a request token that is not header-safe. The request token is wiped on drop
    /// whether it is accepted or refused.
    pub fn new(transport: T, url: Url, request_token: Vec<u8>) -> Option<Self> {
        let request_token = Zeroizing::new(request_token);
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
            .map_err(|_| IdentityUnavailable::Unreachable)?;
        let fields = if response.is_success() {
            read_object(&response.body, Some("value"))
        } else {
            None
        };
        response.body.zeroize();
        // A repeated `value` is no token: another reader could take the other one.
        let Some(fields) = fields.filter(|f| !f.repeated) else {
            return Err(IdentityUnavailable::NoToken);
        };
        let mut jwt = fields.secret.ok_or(IdentityUnavailable::NoToken)?;
        // The text's buffer moves into the token, which wipes it; what is left is empty.
        IdentityToken::new(std::mem::take(&mut *jwt).into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{Response, TransportError};

    /// Compiles only for a buffer the `zeroize` crate wipes on drop.
    fn wiped(_: &Zeroizing<Vec<u8>>) {}

    struct Silent;

    impl Transport for Silent {
        fn send(&mut self, _: &Request<'_>) -> Result<Response, TransportError> {
            Err(TransportError::never_connected("silent"))
        }
    }

    #[test]
    fn every_buffer_that_holds_a_token_is_wiped_storage() {
        // `e30` is `{}`; each part is non-empty base64url.
        let token = IdentityToken::new(b"e30.e30.e30".to_vec()).unwrap();
        wiped(&token.jwt);
        wiped(&token.payload);
        let url = Url::parse("https://pipelines.example.com/idtoken").unwrap();
        let github = GithubIdentity::new(Silent, url, b"request-token".to_vec()).unwrap();
        wiped(&github.request_token);
        wiped(&base64url(b"e30").unwrap());
    }

    #[test]
    fn a_silent_platform_gives_no_token() {
        let url = Url::parse("https://pipelines.example.com/idtoken").unwrap();
        let mut github = GithubIdentity::new(Silent, url, b"request-token".to_vec()).unwrap();
        assert_eq!(
            github.identity_token("aud").err(),
            Some(IdentityUnavailable::Unreachable)
        );
    }
}
