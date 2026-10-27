// SPDX-License-Identifier: Apache-2.0
//! The one place the crate does I/O: a [`Transport`] sends a [`Request`] and returns the
//! [`Response`]. Everything above it is testable without a network, and the secrets a request
//! carries (a bearer token, the signature in a presigned URL) have types that cannot be printed.

use std::fmt;

use crate::url::Url;

/// Sends one request. An adapter enforces the transport invariants: `https` only, redirects off,
/// no environment proxy settings, and a bearer token only on the requests that carry one.
pub trait Transport {
    /// Sends `request` and returns the answer of the server, whatever its status. An `Err` is a
    /// failure to get an answer at all.
    fn send(&mut self, request: &Request<'_>) -> Result<Response, TransportError>;
}

/// Bytes that must never be printed or copied: neither `Debug` nor `Display` nor `Clone`, and
/// overwritten when dropped.
pub struct Secret(Vec<u8>);

impl Secret {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// `Bearer <token>`, built here so the only copy of the token outside the grant is in a
    /// buffer that is overwritten on drop. `None` unless the token is one or more visible ASCII
    /// characters, which is all a header value may carry.
    pub fn bearer(token: &[u8]) -> Option<Self> {
        let visible = !token.is_empty() && token.iter().all(u8::is_ascii_graphic);
        visible.then(|| {
            let mut value = Vec::with_capacity(7 + token.len());
            value.extend_from_slice(b"Bearer ");
            value.extend_from_slice(token);
            Self(value)
        })
    }

    /// The bytes. Only an adapter writing them to the wire has a reason to read them.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

/// The value of a header: text that is safe to print, or a [`Secret`].
pub enum HeaderValue {
    Plain(String),
    Secret(Secret),
}

pub struct Header {
    pub name: String,
    pub value: HeaderValue,
}

impl Header {
    pub fn plain(name: &str, value: &str) -> Self {
        Self {
            name: name.to_owned(),
            value: HeaderValue::Plain(value.to_owned()),
        }
    }

    /// The `authorization` header carrying `token` as a bearer; `None` for a token that is not
    /// header-safe (see [`Secret::bearer`]).
    pub fn bearer(token: &[u8]) -> Option<Self> {
        Secret::bearer(token).map(|secret| Self {
            name: "authorization".to_owned(),
            value: HeaderValue::Secret(secret),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

/// One request. The body is borrowed, so a file is never copied to be sent. Its `Debug` shows
/// the method, the URL without its query, the header names with plain values, and the length of
/// the body: never a secret.
pub struct Request<'a> {
    pub method: Method,
    pub url: Url,
    pub headers: Vec<Header>,
    pub body: &'a [u8],
}

impl fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<String> = self
            .headers
            .iter()
            .map(|h| match &h.value {
                HeaderValue::Plain(v) => format!("{}: {v}", h.name),
                HeaderValue::Secret(_) => format!("{}: <redacted>", h.name),
            })
            .collect();
        f.debug_struct("Request")
            .field("method", &self.method.as_str())
            .field("url", &self.url.redacted())
            .field("headers", &headers)
            .field("body_len", &self.body.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// The first header called `name`, compared without regard to letter case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// How far a request got when it failed. A mutating call that failed at the transport is
/// `Unknown` in both cases (the seam says so); the difference is kept for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// No connection was made: nothing was written.
    NeverConnected,
    /// The request may have been written, and no usable answer came back (a timeout, a reset,
    /// a truncated or unreadable body).
    NoUsableAnswer,
}

/// A failure to get an answer. The message is built by the adapter from redacted parts and
/// never carries a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportError {
    kind: TransportKind,
    message: String,
}

impl TransportError {
    pub fn never_connected(message: impl Into<String>) -> Self {
        Self {
            kind: TransportKind::NeverConnected,
            message: message.into(),
        }
    }

    pub fn no_usable_answer(message: impl Into<String>) -> Self {
        Self {
            kind: TransportKind::NoUsableAnswer,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> TransportKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            TransportKind::NeverConnected => write!(f, "no connection: {}", self.message),
            TransportKind::NoUsableAnswer => write!(f, "no usable answer: {}", self.message),
        }
    }
}

impl std::error::Error for TransportError {}
