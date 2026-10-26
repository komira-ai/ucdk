// SPDX-License-Identifier: Apache-2.0
//! A session in safe Rust: one ledger source and a holder name, and the verb over protobuf
//! bytes. The functions that cross the C boundary are in `ffi`; nothing here is `unsafe`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, TryLockError};

use prost::Message as _;
use ucdk_api::v1::{Error, ErrorCode, LedgerShowRequest, SessionConfig};
use ucdk_api::{LedgerSource, ledger_show};
#[cfg(feature = "test-clock")]
use ucdk_ledger::ManualClock;
use ucdk_ledger::{Clock, Ledger, MemoryLedger, SystemClock, default_holder};

use crate::status::{self, CallError};

/// The verbs, which share one signature on the C side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Show the ledger document: its token, fence, lease and records. It takes no lease and
    /// changes no file.
    LedgerShow,
}

fn unsupported(what: &str, value: &str) -> CallError {
    CallError::new(status::ABI, format!("this library has no {what} {value:?}"))
}

/// The clock the ledger is judged by: the wall clock, or one fixed at `test_clock_ms`.
#[cfg(feature = "test-clock")]
fn clock_for(test_clock_ms: u64) -> Result<Arc<dyn Clock>, CallError> {
    Ok(if test_clock_ms == 0 {
        Arc::new(SystemClock)
    } else {
        Arc::new(ManualClock::at(test_clock_ms))
    })
}

/// The clock the ledger is judged by: the wall clock. A caller that could fix it would defeat
/// the lease and the fence, so this build refuses `test_clock_ms`.
#[cfg(not(feature = "test-clock"))]
fn clock_for(test_clock_ms: u64) -> Result<Arc<dyn Clock>, CallError> {
    if test_clock_ms == 0 {
        Ok(Arc::new(SystemClock))
    } else {
        Err(CallError::new(
            status::INVALID_ARGUMENT,
            "test_clock_ms needs a library built with the `test-clock` feature",
        ))
    }
}

/// The ledger a session shows.
enum Source {
    /// A ledger held for the life of the session: a memory ledger, or the one a Rust host hands
    /// to [`Session::with_ledger`].
    Ledger(Box<dyn Ledger>),
    /// A ledger file, opened read-only on each call, so that opening a session creates no lock
    /// file and removes no temporary, and a file that cannot be read is the answer of the call
    /// that reads it, not a failure to open the session.
    File {
        path: PathBuf,
        clock: Arc<dyn Clock>,
    },
}

fn source_for(kind: &str, location: &str, clock: Arc<dyn Clock>) -> Result<Source, CallError> {
    match kind {
        "" | "memory" => Ok(Source::Ledger(Box::new(MemoryLedger::with_clock(clock)))),
        "file" if location.is_empty() => Err(CallError::new(
            status::INVALID_ARGUMENT,
            "a file ledger needs ledger_location",
        )),
        "file" => Ok(Source::File {
            path: PathBuf::from(location),
            clock,
        }),
        other => Err(unsupported("ledger kind", other)),
    }
}

fn decode<M: prost::Message + Default>(bytes: &[u8], what: &str) -> Result<M, CallError> {
    M::decode(bytes).map_err(|e| {
        CallError::new(
            status::DECODE,
            format!("the request is not a {what} message: {e}"),
        )
    })
}

/// What a session works on. Only one call at a time holds it.
struct Inner {
    source: Source,
    /// The name a lease is reported as `yours` for.
    holder: String,
}

impl Inner {
    fn run(&mut self, verb: Verb, bytes: &[u8]) -> Result<Vec<u8>, CallError> {
        Ok(match verb {
            Verb::LedgerShow => {
                let LedgerShowRequest {} = decode(bytes, "LedgerShowRequest")?;
                let outcome = match &self.source {
                    Source::Ledger(ledger) => {
                        // The ledger is read as it is, by its own clock.
                        ledger_show(
                            &LedgerSource::Open(ledger.as_ref()),
                            &self.holder,
                            Arc::new(SystemClock),
                        )
                    }
                    Source::File { path, clock } => {
                        ledger_show(&LedgerSource::File(path), &self.holder, Arc::clone(clock))
                    }
                };
                outcome.encode_to_vec()
            }
        })
    }
}

/// An open session: a ledger source and the holder name a lease is compared with.
///
/// A call holds the session's lock without waiting, so a second call at the same time is refused
/// with `BUSY`. A panic inside a call leaves the lock poisoned, and every later call on that
/// session is refused with `PANIC`.
pub struct Session {
    inner: Mutex<Inner>,
}

impl Session {
    /// Opens a session from a `SessionConfig` message. A file ledger is not opened here: it is
    /// opened, read-only, by each call that shows it.
    ///
    /// # Errors
    ///
    /// `DECODE` for bytes that are not a config, `ABI` for a ledger kind this library does not
    /// have, and `INVALID_ARGUMENT` for a file ledger without a location or a `test_clock_ms`
    /// in a build without the `test-clock` feature.
    pub fn open(config: &[u8]) -> Result<Self, CallError> {
        let config: SessionConfig = decode(config, "SessionConfig")?;
        let clock = clock_for(config.test_clock_ms)?;
        let source = source_for(&config.ledger_kind, &config.ledger_location, clock)?;
        let session = Self::from_source(source);
        Ok(if config.holder.is_empty() {
            session
        } else {
            session.with_holder(config.holder)
        })
    }

    fn from_source(source: Source) -> Self {
        Self {
            inner: Mutex::new(Inner {
                source,
                // A name no lease carries, unless the host names one.
                holder: default_holder(),
            }),
        }
    }

    /// The session, with a lease reported as `yours` when `holder` is the one it names.
    #[must_use]
    pub fn with_holder(mut self, holder: impl Into<String>) -> Self {
        self.inner
            .get_mut()
            .expect("the lock is never poisoned")
            .holder = holder.into();
        self
    }

    /// A session over a ledger the host holds: how a Rust host, or a test with a ledger of its
    /// own, builds one without a config. Hand it to [`crate::into_handle`] to get the C handle.
    pub fn with_ledger(ledger: Box<dyn Ledger>) -> Self {
        Self::from_source(Source::Ledger(ledger))
    }

    /// Runs one verb over the request bytes and returns the response bytes.
    ///
    /// # Errors
    ///
    /// `BUSY` when another call holds the session, `PANIC` when an earlier call panicked, and
    /// `DECODE` when `request` is not the verb's message. A ledger that cannot be read is not
    /// an error: it is the `error` arm of the response.
    pub fn call(&self, verb: Verb, request: &[u8]) -> Result<Vec<u8>, CallError> {
        let mut inner = match self.inner.try_lock() {
            Ok(inner) => inner,
            Err(TryLockError::WouldBlock) => {
                return Err(CallError::new(status::BUSY, "the session is in use"));
            }
            Err(TryLockError::Poisoned(_)) => {
                return Err(CallError::new(
                    status::PANIC,
                    "an earlier call panicked; the session is poisoned and can only be closed",
                ));
            }
        };
        inner.run(verb, request)
    }
}

/// The `Error` message for a failed call.
pub(crate) fn error_bytes(error: &CallError) -> Vec<u8> {
    let code = if matches!(
        error.status,
        status::INVALID_ARGUMENT | status::DECODE | status::TOO_LARGE
    ) {
        ErrorCode::InvalidRequest
    } else {
        ErrorCode::Unspecified
    };
    Error {
        code: code as i32,
        message: error.message.clone(),
    }
    .encode_to_vec()
}
