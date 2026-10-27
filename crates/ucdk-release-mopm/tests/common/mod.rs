// SPDX-License-Identifier: Apache-2.0
//! Shared fixtures: a configuration that passes the check, coordinates, archive facts and a
//! transport that records what it is asked to send.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Duration;

use ucdk_release_mopm::{
    ArchiveFacts, Candidate, Config, Request, Response, Roots, Subdir, Transport, TransportError,
    Url,
};
use ucdk_release_proto::v1::{ArtifactType, Coordinate};

pub const CHANNEL: &str = "mojo";
pub const BUILD: &str = "mojo26.2_0";
pub const MANIFEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
pub const DIGEST: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// The proposed defaults: 180 + 30 + 240 + 15 = 465 seconds of budget, 60 of margin, against a
/// lease of 600 seconds (a bound of 595).
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
        upload_budget: secs(180),
        commit_request_timeout: secs(30),
        poll_deadline: secs(240),
        cleanup_timeout: secs(15),
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
