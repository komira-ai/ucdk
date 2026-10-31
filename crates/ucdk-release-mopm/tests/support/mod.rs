// SPDX-License-Identifier: Apache-2.0
//! Fixtures of the grant tests: identity tokens, a scripted transport that records what it was
//! sent, an identity source that records what it was asked, and a grant source built from them.

#![allow(dead_code)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use ucdk_ledger::ManualClock;
use ucdk_release::codec::GrantPhase;
use ucdk_release_mopm::{
    ActorForm, GITHUB_ISSUER, GrantSourceConfig, IdentitySource, IdentityToken,
    IdentityUnavailable, Method, MopmGrantSource, Request, Response, Transport, TransportError,
};
use ucdk_release_proto::v1::push_identity::Kind;
use ucdk_release_proto::v1::{ArtifactType, Federated, PushIdentity, Repository, Visibility};
use ucdk_release_seams::GrantRequest;

pub const NOW_S: u64 = 1_800_000_000;
pub const NOW_MS: u64 = NOW_S * 1000;
pub const LOCATION: &str = "https://registry.example.com/api/v1";
pub const AUDIENCE: &str = "mopm.example.com";
pub const SUBJECT: &str = "repo:example-org/lib:environment:publish";
pub const MINTED: &str = "CANARY-minted-token-0123456789";

/// `2027-01-15T08:00:00Z` plus `seconds`, in the form mopm writes (within that day).
pub fn iso(seconds: u64) -> String {
    let total = 8 * 3600 + seconds;
    format!(
        "2027-01-15T{:02}:{:02}:{:02}.000Z",
        total / 3600,
        total / 60 % 60,
        total % 60
    )
}

fn b64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
        }
    }
    out
}

/// A JWT-shaped identity token with these claims; the signature is a fixed canary text.
pub fn jwt_of(claims: &Value) -> Vec<u8> {
    format!(
        "{}.{}.CANARY-signature-abc",
        b64(br#"{"alg":"RS256","kid":"k"}"#),
        b64(claims.to_string().as_bytes())
    )
    .into_bytes()
}

/// The claims of a job the declaration of [`repository`] admits.
pub fn claims(jti: &str) -> Value {
    json!({
        "iss": GITHUB_ISSUER,
        "aud": AUDIENCE,
        "sub": SUBJECT,
        "environment": "publish",
        "ref": "refs/heads/main",
        "repository_visibility": "public",
        "repository_id": "9001",
        "iat": NOW_S - 5,
        "nbf": NOW_S - 5,
        "exp": NOW_S + 300,
        "jti": jti,
    })
}

pub fn token(jti: &str) -> Vec<u8> {
    jwt_of(&claims(jti))
}

pub fn identity_token(claims: &Value) -> IdentityToken {
    IdentityToken::new(jwt_of(claims)).unwrap()
}

pub fn federated() -> Federated {
    Federated {
        subject: SUBJECT.to_owned(),
        audience: AUDIENCE.to_owned(),
        exceptional_subject: "repo:example-org/lib:environment:exceptional".to_owned(),
        environment: "publish".to_owned(),
        exceptional_environment: "exceptional".to_owned(),
        pinned_claims: BTreeMap::from([("ref".to_owned(), "refs/heads/main".to_owned())]),
        stage_scope: "publish".to_owned(),
        commit_scope: "publish".to_owned(),
        read_scope: String::new(),
    }
}

pub fn repository_with(visibility: Visibility, declared: Federated) -> Repository {
    Repository {
        r#type: ArtifactType::Conda as i32,
        location: LOCATION.to_owned(),
        visibility: visibility as i32,
        push: Some(PushIdentity {
            kind: Some(Kind::Federated(declared)),
        }),
    }
}

/// A public repository with a federated push identity.
pub fn repository() -> Repository {
    repository_with(Visibility::Public, federated())
}

pub fn request<'a>(
    repository: &'a Repository,
    phase: GrantPhase,
    coordinates: &'a [ucdk_release_proto::v1::Coordinate],
) -> GrantRequest<'a> {
    GrantRequest {
        repository,
        phase,
        coordinates,
        exceptional: false,
    }
}

/// One request the scripted transport was sent.
pub struct Sent {
    pub method: Method,
    pub url: String,
    pub body: Vec<u8>,
    /// `Debug` of the request: the text a log would hold.
    pub debug: String,
    pub headers: Vec<String>,
}

/// What the scripted transport answers next.
pub enum Reply {
    Raw(Result<Response, TransportError>),
    /// A 200 with a minted token for the `packages` of the request body.
    Echo {
        token: String,
        kind: String,
        expires_at: String,
    },
}

pub fn echo(expires_at: &str) -> Reply {
    Reply::Echo {
        token: MINTED.to_owned(),
        kind: "publish".to_owned(),
        expires_at: expires_at.to_owned(),
    }
}

pub fn raw(status: u16, headers: &[(&str, &str)], body: &str) -> Reply {
    Reply::Raw(Ok(crate::common::response(status, headers, body)))
}

#[derive(Clone, Default)]
pub struct Script {
    pub sent: Arc<Mutex<Vec<Sent>>>,
    replies: Arc<Mutex<VecDeque<Reply>>>,
}

impl Script {
    pub fn push(&self, reply: Reply) {
        self.replies.lock().unwrap().push_back(reply);
    }

    pub fn count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    pub fn debugs(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.debug.clone())
            .collect()
    }
}

impl Transport for Script {
    fn send(&mut self, request: &Request<'_>) -> Result<Response, TransportError> {
        self.sent.lock().unwrap().push(Sent {
            method: request.method,
            url: request.url.expose(),
            body: request.body.to_vec(),
            debug: format!("{request:?}"),
            headers: request
                .headers
                .iter()
                .map(|h| h.name.to_ascii_lowercase())
                .collect(),
        });
        let reply = self.replies.lock().unwrap().pop_front();
        match reply {
            None => Err(TransportError::never_connected("no reply queued")),
            Some(Reply::Raw(result)) => result,
            Some(Reply::Echo {
                token,
                kind,
                expires_at,
            }) => {
                let sent: Value = serde_json::from_slice(request.body).unwrap();
                let body = json!({
                    "token": token,
                    "token_kind": kind,
                    "expires_at": expires_at,
                    "packages": sent["packages"],
                });
                Ok(crate::common::response(200, &[], &body.to_string()))
            }
        }
    }
}

/// An identity source that hands out the queued tokens and records the audience it was asked for.
#[derive(Clone, Default)]
pub struct Tokens {
    queue: Arc<Mutex<VecDeque<Vec<u8>>>>,
    pub audiences: Arc<Mutex<Vec<String>>>,
}

impl Tokens {
    pub fn push(&self, jwt: Vec<u8>) {
        self.queue.lock().unwrap().push_back(jwt);
    }

    pub fn asked(&self) -> usize {
        self.audiences.lock().unwrap().len()
    }
}

impl IdentitySource for Tokens {
    fn identity_token(&mut self, audience: &str) -> Result<IdentityToken, IdentityUnavailable> {
        self.audiences.lock().unwrap().push(audience.to_owned());
        let next = self.queue.lock().unwrap().pop_front();
        next.ok_or(IdentityUnavailable("none queued"))
            .and_then(IdentityToken::new)
    }
}

pub struct Rig {
    pub script: Script,
    pub tokens: Tokens,
    pub clock: ManualClock,
}

pub type Source = MopmGrantSource<Script, Tokens, ManualClock>;

pub fn source_for(actor: ActorForm, clock: &ManualClock) -> (Source, Rig) {
    let script = Script::default();
    let tokens = Tokens::default();
    let setup = GrantSourceConfig {
        config: crate::common::config(),
        location: LOCATION.to_owned(),
        issuer: GITHUB_ISSUER.to_owned(),
        actor,
    };
    let source = MopmGrantSource::new(setup, script.clone(), tokens.clone(), clock.clone());
    (
        source,
        Rig {
            script,
            tokens,
            clock: clock.clone(),
        },
    )
}

/// A source of the configured CI actor, the clock at [`NOW_MS`].
pub fn source() -> (Source, Rig) {
    source_for(ActorForm::ConfiguredCi, &ManualClock::at(NOW_MS))
}
