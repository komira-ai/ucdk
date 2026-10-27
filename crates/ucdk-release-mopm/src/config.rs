// SPDX-License-Identifier: Apache-2.0
//! The configuration of the mopm channel: a plain struct the caller builds from its flags. No
//! field has a default (a default registry or a default lease could silently differ from what
//! the caller means), nothing is read from the environment, and the trust anchors are one named
//! bundle file.

use std::fmt;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use ucdk_ledger::MAX_CLOCK_SKEW;

use crate::url::{Origin, Url, UrlError, parse_authority};

/// How long mopm's token lives from its exchange. The call a token runs under must end before
/// it does.
pub const MOPM_TOKEN_TTL: Duration = Duration::from_secs(600);

/// The most package names a page of versions may ask for (mopm answers 1 to 100).
pub const MAX_PAGE: u32 = 100;

/// Where the trust anchors of a request that carries a bearer token come from: one PEM file
/// named by the caller. The platform's certificate loader is never used, because it can read
/// certificate variables from the job's environment, and a bearer token must not depend on them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Roots {
    Bundle(PathBuf),
}

/// Why a trust bundle was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootsError {
    /// The file could not be read as text.
    Unreadable(io::ErrorKind),
    /// The file holds no certificate: a bundle that trusts nothing would fail every request.
    NoCertificate,
    /// A certificate block is not closed, or a block is closed that was never opened.
    Unterminated,
}

impl fmt::Display for RootsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RootsError::Unreadable(kind) => write!(f, "the trust bundle cannot be read: {kind}"),
            RootsError::NoCertificate => f.write_str("the trust bundle holds no certificate"),
            RootsError::Unterminated => f.write_str("the trust bundle has an unbalanced block"),
        }
    }
}

impl std::error::Error for RootsError {}

const BEGIN: &str = "-----BEGIN CERTIFICATE-----";
const END: &str = "-----END CERTIFICATE-----";

impl Roots {
    /// The text of the bundle: exactly the named file, and nothing the environment says. It holds
    /// at least one certificate block and every block is closed.
    pub fn bundle_pem(&self) -> Result<String, RootsError> {
        let Roots::Bundle(path) = self;
        let text = fs::read_to_string(path).map_err(|e| RootsError::Unreadable(e.kind()))?;
        let (begins, ends) = (text.matches(BEGIN).count(), text.matches(END).count());
        if begins == 0 {
            return Err(RootsError::NoCertificate);
        }
        if begins != ends {
            return Err(RootsError::Unterminated);
        }
        Ok(text)
    }
}

/// An explicit proxy, `host:port`. The environment's proxy settings are never read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proxy {
    host: String,
    port: u16,
}

impl Proxy {
    pub fn parse(text: &str) -> Result<Self, UrlError> {
        let (host, port) = parse_authority(text, None)?;
        Ok(Self { host, port })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Why a configuration is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// The registry location carries a query.
    RegistryQuery,
    /// A duration that must be positive is zero.
    ZeroDuration(&'static str),
    /// `newest_page` is outside 1 to [`MAX_PAGE`].
    Page(u32),
    /// The lease is not longer than the clock-skew allowance, so no call fits in it.
    LeaseTooShort { lease_ttl: Duration },
    /// One call, with the margin, does not fit under the token and the lease it runs under.
    Budget {
        call: Call,
        needed: Duration,
        bound: Duration,
    },
}

/// The three calls of the staged shape that spend time. Each is its own seam call, so each runs
/// under its own grant (a fresh exchange, ten minutes) and its own lease renewal: the bound
/// applies to one of them at a time, never to their sum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    /// The group read, the create and every `PUT` with its retries.
    Stage,
    /// The commit request and the wait for visibility.
    Commit,
    /// The `DELETE` of an abort and the status read after it.
    Abort,
}

impl Call {
    /// Every call, in the order the engine makes them.
    pub const ALL: [Call; 3] = [Call::Stage, Call::Commit, Call::Abort];

    fn name(self) -> &'static str {
        match self {
            Call::Stage => "stage",
            Call::Commit => "commit",
            Call::Abort => "abort",
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::RegistryQuery => f.write_str("the registry location has a query"),
            ConfigError::ZeroDuration(name) => write!(f, "{name} must be more than zero"),
            ConfigError::Page(n) => write!(f, "newest_page is {n}; it must be 1 to {MAX_PAGE}"),
            ConfigError::LeaseTooShort { lease_ttl } => write!(
                f,
                "the lease of {}s is not longer than the clock-skew allowance of {}s",
                lease_ttl.as_secs(),
                MAX_CLOCK_SKEW.as_secs()
            ),
            ConfigError::Budget {
                call,
                needed,
                bound,
            } => write!(
                f,
                "a {} call may take {}s with its margin, but the token and the lease allow {}s",
                call.name(),
                needed.as_secs(),
                bound.as_secs()
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Everything the publisher is told. There is no `Default`: each field is the caller's decision.
#[derive(Clone)]
pub struct Config {
    /// The channel the publisher is built for; a coordinate of another channel is refused.
    pub channel: String,
    /// The registry's API base, with no query.
    pub registry: Url,
    /// Origins other than the registry's that a file URL (a presigned upload, a download) may
    /// name; any other origin is refused before the request.
    pub file_hosts: Vec<Origin>,
    pub roots: Roots,
    pub proxy: Option<Proxy>,
    pub request_timeout: Duration,
    pub connect_timeout: Duration,
    /// How often an idempotent call (a `GET` or the `PUT` of a file) is retried.
    pub retries: u32,
    pub poll_interval: Duration,
    /// The time-to-live of the engine's lease guard, set by the caller from the value the engine
    /// uses.
    pub lease_ttl: Duration,
    /// What a `stage` call may spend: the group read, the create, every `PUT` and every retry
    /// of them.
    pub upload_budget: Duration,
    pub commit_request_timeout: Duration,
    /// How long a `commit` call waits for the group to become visible. Zero returns as soon as
    /// the commit is accepted.
    pub poll_deadline: Duration,
    /// What an `abort` call may spend: the `DELETE` and the status read after it.
    pub abort_timeout: Duration,
    /// The exchange-to-call gap and one connect timeout.
    pub margin: Duration,
    /// The longest `Retry-After` the publisher turns into a wait; a longer one is a refusal.
    pub retry_after_ceiling: Duration,
    /// How often the identical create is repeated while the group is still `reserving`.
    pub reserving_repeats: u32,
    /// The page size of a versions read, 1 to [`MAX_PAGE`].
    pub newest_page: u32,
    /// The most versions of one package `newest` reads; more is an error, never a truncation.
    pub newest_versions: usize,
}

impl Config {
    /// What one `call` may spend: `upload_budget` for a stage, `commit_request_timeout +
    /// poll_deadline` for a commit, `abort_timeout` for an abort. Saturating, so an absurd
    /// setting fails the check instead of wrapping.
    pub fn budget(&self, call: Call) -> Duration {
        match call {
            Call::Stage => self.upload_budget,
            Call::Commit => self
                .commit_request_timeout
                .saturating_add(self.poll_deadline),
            Call::Abort => self.abort_timeout,
        }
    }

    /// The longest a call may run: the shorter of the token's life and the lease less the
    /// clock-skew allowance.
    pub fn bound(&self) -> Result<Duration, ConfigError> {
        let lease = self
            .lease_ttl
            .checked_sub(MAX_CLOCK_SKEW)
            .filter(|left| !left.is_zero())
            .ok_or(ConfigError::LeaseTooShort {
                lease_ttl: self.lease_ttl,
            })?;
        Ok(lease.min(MOPM_TOKEN_TTL))
    }

    /// The construction-time check, before anything is sent: the registry has no query, the
    /// timeouts are positive, the page size is one mopm answers, and
    /// `budget(call) + margin <= min(MOPM_TOKEN_TTL, lease_ttl - skew)` for each of the stage,
    /// commit and abort calls, so no call outlives the token or the lease it runs under. The
    /// calls are separate seam calls with a grant exchange and a lease renewal before each, so
    /// the bound is per call and their sum is not checked.
    pub fn check(&self) -> Result<(), ConfigError> {
        if self.registry.has_query() {
            return Err(ConfigError::RegistryQuery);
        }
        let positive = [
            ("request_timeout", self.request_timeout),
            ("connect_timeout", self.connect_timeout),
            ("poll_interval", self.poll_interval),
            ("upload_budget", self.upload_budget),
            ("commit_request_timeout", self.commit_request_timeout),
        ];
        if let Some((name, _)) = positive.into_iter().find(|(_, d)| d.is_zero()) {
            return Err(ConfigError::ZeroDuration(name));
        }
        if !(1..=MAX_PAGE).contains(&self.newest_page) {
            return Err(ConfigError::Page(self.newest_page));
        }
        let bound = self.bound()?;
        for call in Call::ALL {
            let needed = self.budget(call).saturating_add(self.margin);
            if needed > bound {
                return Err(ConfigError::Budget {
                    call,
                    needed,
                    bound,
                });
            }
        }
        Ok(())
    }

    /// Whether a file URL may be fetched: its origin is the registry's or one of `file_hosts`.
    pub fn allows_file_origin(&self, url: &Url) -> bool {
        let origin = url.origin();
        origin == self.registry.origin() || self.file_hosts.contains(origin)
    }
}
