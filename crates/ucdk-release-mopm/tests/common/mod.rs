// SPDX-License-Identifier: Apache-2.0
//! Shared fixtures: a configuration that passes the check, coordinates, archive facts and a
//! transport that records what it is asked to send.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use ucdk_core::ManualClock;
use ucdk_release_mopm::{
    ArchiveFacts, Caller, Candidate, Config, HeaderValue, Key, Member, Method, Origin, Registry,
    Request, Response, Roots, SetFile, Subdir, Transport, TransportError, Url, group_id, members,
};
use ucdk_release_proto::v1::{ArtifactType, Coordinate};
use ucdk_release_seams::{SeamError, StageAnswer};

pub const CHANNEL: &str = "mojo";
pub const BUILD: &str = "mojo26.2_0";
pub const MANIFEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub const DIGEST: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// The proposed defaults: a stage of 300 seconds, a commit of 30 + 420, an abort of 15, each with
/// 60 of margin, against a lease of 600 seconds (a bound of 595 for each call).
pub fn config() -> Config {
    Config {
        channel: CHANNEL.to_owned(),
        registry: Url::parse("https://registry.example.com/api/v1").unwrap(),
        file_hosts: Vec::new(),
        roots: Roots::Bundle(PathBuf::from("roots.pem")),
        proxy: None,
        request_timeout: secs(30),
        connect_timeout: secs(10),
        retries: 2,
        poll_interval: secs(2),
        lease_ttl: secs(600),
        upload_budget: secs(300),
        commit_request_timeout: secs(30),
        poll_deadline: secs(420),
        abort_timeout: secs(15),
        margin: secs(60),
        retry_after_ceiling: secs(600),
        reserving_repeats: 3,
        newest_page: 100,
        newest_versions: 1000,
    }
}

pub fn coordinate(name: &str, version: &str, platform: &str) -> Coordinate {
    Coordinate {
        channel: CHANNEL.to_owned(),
        r#type: ArtifactType::Conda as i32,
        name: name.to_owned(),
        version: version.to_owned(),
        platform: platform.to_owned(),
    }
}

pub fn file_name(name: &str, version: &str) -> String {
    format!("mojo-{name}-{version}-{BUILD}.conda")
}

/// Archive facts that agree with `name`, `version` and `subdir`.
pub fn facts(name: &str, version: &str, subdir: Subdir) -> ArchiveFacts {
    ArchiveFacts {
        index_name: format!("mojo-{name}"),
        index_version: version.to_owned(),
        index_subdir: subdir.as_str().to_owned(),
        index_build: BUILD.to_owned(),
        manifest_name: name.to_owned(),
        manifest_version: Some(version.to_owned()),
        manifest_sha256: MANIFEST.to_owned(),
        native_library: None,
    }
}

/// A candidate of 1000 bytes for `coordinate`, whose file name and archive agree with it.
pub struct Owned {
    pub coordinate: Coordinate,
    pub file_name: String,
    pub size: u64,
    pub sha256: String,
    pub archive: ArchiveFacts,
}

impl Owned {
    pub fn new(name: &str, version: &str, platform: &str) -> Self {
        let subdir = Subdir::of_platform(platform).unwrap_or(Subdir::Noarch);
        Self {
            coordinate: coordinate(name, version, platform),
            file_name: file_name(name, version),
            size: 1000,
            sha256: DIGEST.to_owned(),
            archive: facts(name, version, subdir),
        }
    }

    pub fn candidate(&self) -> Candidate<'_> {
        Candidate {
            coordinate: &self.coordinate,
            file_name: &self.file_name,
            size: self.size,
            sha256: &self.sha256,
            archive: &self.archive,
        }
    }
}

pub fn candidates(owned: &[Owned]) -> Vec<Candidate<'_>> {
    owned.iter().map(Owned::candidate).collect()
}

pub fn response(status: u16, headers: &[(&str, &str)], body: &str) -> Response {
    Response {
        status,
        headers: headers
            .iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect(),
        body: body.as_bytes().to_vec(),
    }
}

/// A mopm error body: `{"error": {"code": ..., "message": ..., "hint": ...}}`.
pub fn error_body(code: &str) -> String {
    format!(r#"{{"error":{{"code":"{code}","message":"m","hint":"h"}}}}"#)
}

/// Records every request it is asked to send (as the redacted text of its `Debug`) and answers
/// from a queue.
#[derive(Default)]
pub struct Recording {
    pub sent: Vec<String>,
    pub replies: VecDeque<Result<Response, TransportError>>,
}

impl Transport for Recording {
    fn send(&mut self, request: &Request<'_>) -> Result<Response, TransportError> {
        self.sent.push(format!("{request:?}"));
        self.replies
            .pop_front()
            .unwrap_or_else(|| Err(TransportError::never_connected("no reply queued")))
    }
}

pub type Reply = Result<Response, TransportError>;

/// One request as the fake saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    pub method: Method,
    pub path: String,
    /// The whole URL, query included: the fake is a test double and holds what a real one would not
    /// print.
    pub url: String,
    pub query: bool,
    pub body: String,
    /// The request carried a bearer token.
    pub authorized: bool,
}

/// A registry written by hand: each route is a method and a path with the replies it gives in
/// order (the last one repeats), a request to a path with no route is never connected, and every
/// request is logged. A route may also move a clock when it is hit, to spend a time budget.
#[derive(Default)]
pub struct Fake {
    pub log: Vec<Logged>,
    routes: Vec<(Method, String, VecDeque<Reply>)>,
    bumps: Vec<(Method, String, ManualClock, Duration)>,
}

impl Fake {
    pub fn on(mut self, method: Method, path: &str, replies: Vec<Reply>) -> Self {
        self.routes.push((method, path.to_owned(), replies.into()));
        self
    }

    pub fn bump(mut self, method: Method, path: &str, clock: &ManualClock, by: Duration) -> Self {
        self.bumps
            .push((method, path.to_owned(), clock.clone(), by));
        self
    }

    /// How many requests of `method` went to `path`.
    pub fn count(&self, method: Method, path: &str) -> usize {
        self.log
            .iter()
            .filter(|l| l.method == method && l.path == path)
            .count()
    }

    /// The log as `METHOD path` lines.
    pub fn lines(&self) -> Vec<String> {
        self.log
            .iter()
            .map(|l| format!("{} {}", l.method.as_str(), l.path))
            .collect()
    }
}

impl Transport for Fake {
    fn send(&mut self, request: &Request<'_>) -> Result<Response, TransportError> {
        let path = request.url.path().to_owned();
        self.log.push(Logged {
            method: request.method,
            path: path.clone(),
            url: request.url.expose(),
            query: request.url.has_query(),
            body: String::from_utf8_lossy(request.body).into_owned(),
            authorized: request
                .headers
                .iter()
                .any(|h| matches!(h.value, HeaderValue::Secret(_))),
        });
        for (method, at, clock, by) in &self.bumps {
            if *method == request.method && *at == path {
                clock.advance(*by);
            }
        }
        let route = self
            .routes
            .iter_mut()
            .find(|(method, at, _)| *method == request.method && *at == path);
        let reply = match route {
            Some((_, _, replies)) if replies.len() > 1 => replies.pop_front(),
            Some((_, _, replies)) => replies.front().cloned(),
            None => None,
        };
        reply.unwrap_or_else(|| Err(TransportError::never_connected("no route")))
    }
}

pub const ACTOR: &str = "u:1234";
pub const TOKEN: &[u8] = b"CANARY-token-0123";
pub const GROUPS: &str = "/api/v1/groups";

pub fn caller() -> Caller<'static> {
    Caller {
        actor: ACTOR,
        token: TOKEN,
    }
}

pub fn group_path(id: &str) -> String {
    format!("{GROUPS}/{id}")
}

pub fn record_path(name: &str, version: &str) -> String {
    format!("/api/v1/packages/{name}/{version}")
}

/// The configuration of the group tests: the proposed defaults, and `files.example.com` as a file
/// host.
pub fn group_config() -> Config {
    let mut c = config();
    c.file_hosts = vec![Origin::parse("https://files.example.com").unwrap()];
    c
}

pub fn registry(fake: Fake, clock: &ManualClock) -> Registry<Fake> {
    Registry::new(fake, group_config(), Box::new(clock.clone()))
}

/// The members of the set `owned`.
pub fn set(owned: &[Owned]) -> Vec<Member> {
    members(CHANNEL, &candidates(owned)).unwrap()
}

/// The bytes of every file of the tests: 1000 bytes, the size `Owned` declares.
pub fn file_bytes() -> Vec<u8> {
    vec![7; 1000]
}

pub fn set_files<'a>(owned: &'a [Owned], bytes: &'a [u8]) -> Vec<SetFile<'a>> {
    owned
        .iter()
        .map(|o| SetFile {
            name: &o.coordinate.name,
            version: &o.coordinate.version,
            subdir: Subdir::of_platform(&o.coordinate.platform).unwrap(),
            bytes,
        })
        .collect()
}

/// One member `alpha 1.0.0` with a file for each of two subdirectories.
pub fn alpha() -> Vec<Owned> {
    vec![
        Owned::new("alpha", "1.0.0", "linux-x86_64"),
        Owned::new("alpha", "1.0.0", "any"),
    ]
}

pub fn put_url(file: &str) -> String {
    format!("https://files.example.com/store/{file}?X-Amz-Signature=CANARYSIG")
}

/// The files of `members` for a create answer: stored when their subdirectory is in `stored`,
/// otherwise with a `PUT` URL and header.
fn upload_files(member: &Member, stored: &[&str]) -> Vec<Value> {
    member
        .files
        .iter()
        .map(|f| {
            let subdir = f.subdir.as_str();
            if stored.contains(&subdir) {
                json!({"subdir": subdir, "filename": f.file_name, "upload_id": "u", "uploaded": true})
            } else {
                json!({
                    "subdir": subdir,
                    "filename": f.file_name,
                    "upload_id": "u",
                    "uploaded": false,
                    "put_url": put_url(&format!("{subdir}/{}", f.file_name)),
                    "put_headers": {"x-amz-checksum-sha256": "abc"},
                })
            }
        })
        .collect()
}

/// The answer to a create of `members` as group `id` in `status`.
pub fn created_json(id: &str, status: &str, members: &[Member], stored: &[&str]) -> Value {
    let members: Vec<Value> = members
        .iter()
        .map(|m| {
            let files = if status == "open" {
                upload_files(m, stored)
            } else {
                Vec::new()
            };
            json!({"name": m.name, "version": m.version, "action": "new", "files": files})
        })
        .collect();
    json!({"group_id": id, "status": status, "attempt": 1, "deadline_at": "2026-12-01T00:00:00Z", "members": members})
}

/// The answer to a read of group `id`: the declared files of `members`, with `uploaded` only while
/// the group is open.
pub fn group_json(id: &str, status: &str, members: &[Member], stored: &[&str]) -> Value {
    let members: Vec<Value> = members
        .iter()
        .map(|m| {
            let files: Vec<Value> = m
                .files
                .iter()
                .map(|f| {
                    let mut file = json!({
                        "subdir": f.subdir.as_str(),
                        "filename": f.file_name,
                        "sha256": f.sha256,
                        "size": f.size,
                    });
                    if status == "open" {
                        file["uploaded"] = json!(stored.contains(&f.subdir.as_str()));
                    }
                    file
                })
                .collect();
            json!({"name": m.name, "version": m.version, "state": "reserved", "files": files})
        })
        .collect();
    json!({
        "group_id": id,
        "status": status,
        "attempt": 1,
        "created_at": "2026-12-01T00:00:00Z",
        "deadline_at": "2026-12-01T12:00:00Z",
        "member_count": members.len(),
        "members": members,
    })
}

/// A version record of `name` `version` in `state` with a file for each of `subdirs`.
pub fn record_json(name: &str, version: &str, state: &str, subdirs: &[&str]) -> Value {
    let files: Vec<Value> = subdirs
        .iter()
        .map(|subdir| {
            let mut file = json!({
                "subdir": subdir,
                "filename": file_name(name, version),
                "sha256": DIGEST,
                "size": 1000,
            });
            if state != "tombstoned" {
                file["url"] = json!(format!("https://files.example.com/{subdir}/f.conda"));
            }
            file
        })
        .collect();
    json!({"name": name, "version": version, "state": state, "files": files})
}

pub fn json_reply(status: u16, value: &Value) -> Reply {
    Ok(response(status, &[], &value.to_string()))
}

pub fn error_reply(status: u16, code: &str) -> Reply {
    Ok(response(status, &[], &error_body(code)))
}

/// A `group_conflict` with the given reason, or none.
pub fn conflict_reply(reason: Option<&str>) -> Reply {
    let mut body = json!({"error": {"code": "group_conflict", "message": "m", "hint": "h"}});
    if let Some(reason) = reason {
        body["error"]["reason"] = json!(reason);
    }
    json_reply(409, &body)
}

/// An answer that never came: the request may have been written.
pub fn lost() -> Reply {
    Err(TransportError::no_usable_answer("the answer was lost"))
}

pub fn ok(status: u16) -> Reply {
    Ok(response(status, &[], ""))
}

/// The members of `alpha()` and the id of their group for `ACTOR`.
pub fn world() -> (Vec<Owned>, Vec<Member>, String) {
    let owned = alpha();
    let members = set(&owned);
    let id = group_id(ACTOR, &members);
    (owned, members, id)
}

/// The path a file of `alpha 1.0.0` is put to.
pub fn put_path(subdir: &str) -> String {
    format!("/store/{subdir}/{}", file_name("alpha", "1.0.0"))
}

pub fn err_kind(error: &SeamError) -> &'static str {
    match error {
        SeamError::Refused(_) => "refused",
        SeamError::Permanent(_) => "permanent",
        SeamError::Transient(_) => "transient",
        SeamError::RateLimited(_) => "rate_limited",
        SeamError::InFlight(_) => "in_flight",
        SeamError::Unknown(_) => "unknown",
        SeamError::GrantRefused(_) => "grant_refused",
    }
}

/// What a `stage` answered, in a word.
pub fn stage_kind(result: &Result<StageAnswer, SeamError>) -> &'static str {
    match result {
        Ok(StageAnswer::Staged) => "staged",
        Ok(StageAnswer::Duplicate) => "duplicate",
        Err(error) => err_kind(error),
    }
}

/// The coordinate key of `alpha 1.0.0` in `subdir`.
pub fn key(subdir: Subdir) -> Key {
    Key {
        name: "alpha".to_owned(),
        version: "1.0.0".to_owned(),
        subdir,
    }
}

/// The package file of the sparse index: each record on a line of its own with the record version
/// `v`, and a newline at the end.
pub fn index_body(records: &[Value]) -> String {
    records
        .iter()
        .map(|r| {
            let mut line = r.clone();
            line["v"] = json!(1);
            format!("{line}\n")
        })
        .collect()
}

/// A text reply.
pub fn text_reply(status: u16, body: &str) -> Reply {
    Ok(response(status, &[], body))
}
