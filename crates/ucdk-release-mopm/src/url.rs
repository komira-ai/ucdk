// SPDX-License-Identifier: Apache-2.0
//! `https` locations: an [`Origin`] (scheme, host, port), a [`Url`] whose query may carry a
//! presigned signature and so cannot be printed, and the percent-encoding of one path segment.

use std::fmt;

/// Why a text is not an `https` location this crate accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlError {
    /// The text does not begin with the `https` scheme. Plain `http` is never accepted.
    NotHttps,
    /// A character outside visible ASCII (a blank, a control character, a non-ASCII letter).
    Character,
    /// A host that is empty or holds anything but letters, digits, dots and hyphens (user
    /// information and bracketed addresses included).
    Host,
    /// A port that is not a number from 1 to 65535.
    Port,
    /// A fragment, which no request carries.
    Fragment,
    /// A location with a path or a query where only an origin is allowed.
    NotOrigin,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UrlError::NotHttps => "not an https location",
            UrlError::Character => "a character that is not visible ASCII",
            UrlError::Host => "a host that is empty or not letters, digits, dots and hyphens",
            UrlError::Port => "a port that is not 1 to 65535",
            UrlError::Fragment => "a fragment",
            UrlError::NotOrigin => "a path or query where an origin is required",
        })
    }
}

impl std::error::Error for UrlError {}

/// `host[:port]` as a lower-case host and a port. `default_port` is used when the text has none.
pub(crate) fn parse_authority(
    text: &str,
    default_port: Option<u16>,
) -> Result<(String, u16), UrlError> {
    let (host, port) = match text.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (text, None),
    };
    let valid = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    if !valid {
        return Err(UrlError::Host);
    }
    let port = match port {
        Some(digits) => digits
            .bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| digits.parse::<u16>().ok())
            .flatten()
            .filter(|p| *p != 0),
        None => default_port,
    };
    Ok((host.to_ascii_lowercase(), port.ok_or(UrlError::Port)?))
}

/// The scheme, host and port of an `https` location. The scheme is always `https`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Origin {
    host: String,
    port: u16,
}

impl Origin {
    /// An `https` host with an optional port, and at most a trailing slash after it.
    pub fn parse(text: &str) -> Result<Self, UrlError> {
        let url = Url::parse(text)?;
        if url.path != "/" || url.query.is_some() {
            return Err(UrlError::NotOrigin);
        }
        Ok(url.origin)
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.port == 443 {
            write!(f, "https://{}", self.host)
        } else {
            write!(f, "https://{}:{}", self.host, self.port)
        }
    }
}

/// An `https` URL. Neither `Debug` nor `Display`: its query is where a presigned signature
/// lives. [`Url::redacted`] is the form that may be printed, [`Url::expose`] the form that goes
/// on the wire.
#[derive(Clone, PartialEq, Eq)]
pub struct Url {
    origin: Origin,
    path: String,
    query: Option<String>,
}

impl Url {
    pub fn parse(text: &str) -> Result<Self, UrlError> {
        if !text.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(UrlError::Character);
        }
        let rest = text.strip_prefix("https://").ok_or(UrlError::NotHttps)?;
        if rest.contains('#') {
            return Err(UrlError::Fragment);
        }
        let (authority, tail) = rest.split_at(rest.find(['/', '?']).unwrap_or(rest.len()));
        let (host, port) = parse_authority(authority, Some(443))?;
        let (path, query) = match tail.split_once('?') {
            Some((path, query)) => (path, Some(query.to_owned())),
            None => (tail, None),
        };
        let path = if path.is_empty() { "/" } else { path };
        Ok(Self {
            origin: Origin { host, port },
            path: path.to_owned(),
            query,
        })
    }

    pub fn origin(&self) -> &Origin {
        &self.origin
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn has_query(&self) -> bool {
        self.query.is_some()
    }

    /// This URL with each of `segments` appended to its path, each percent-encoded as one path
    /// segment (so a `/` inside one stays inside it), and with no query.
    #[must_use]
    pub fn join(&self, segments: &[&str]) -> Url {
        let mut path = self.path.trim_end_matches('/').to_owned();
        for segment in segments {
            path.push('/');
            path.push_str(&encode_segment(segment));
        }
        Url {
            origin: self.origin.clone(),
            path,
            query: None,
        }
    }

    /// The URL with its query replaced by `<redacted>`: safe to print and to put in an error.
    pub fn redacted(&self) -> String {
        let query = if self.query.is_some() {
            "?<redacted>"
        } else {
            ""
        };
        format!("{}{}{query}", self.origin, self.path)
    }

    /// The full URL, query included, for the adapter that writes it to the wire.
    pub fn expose(&self) -> String {
        match &self.query {
            Some(query) => format!("{}{}?{query}", self.origin, self.path),
            None => format!("{}{}", self.origin, self.path),
        }
    }
}

/// One path segment, percent-encoded: letters, digits and `-._~` stay, every other byte is
/// `%XX` in upper case, so no character of the input can end the segment.
pub fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}
