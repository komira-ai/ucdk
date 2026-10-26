// SPDX-License-Identifier: Apache-2.0
//! A session in safe Rust: one provider, one ledger and a cell, and the five verbs over protobuf
//! bytes. The functions that cross the C boundary are in `ffi`; nothing here is `unsafe`.

use std::sync::{Arc, Mutex, TryLockError};

use prost::Message as _;
use ucdk_api::v1::{
    self, ApplyRequest, CredentialSource, DestroyRequest, Error, ErrorCode, LeaksRequest, Mode,
    PlanRequest, SessionConfig, StatusRequest,
};
use ucdk_api::{Approved, RequestError, catalog_response};
use ucdk_engine::{Engine, Outcome, Request};
#[cfg(feature = "test-clock")]
use ucdk_ledger::ManualClock;
use ucdk_ledger::{Clock, FileLedger, Ledger, MemoryLedger, SystemClock};
use ucdk_plan::{Plan, lower};
use ucdk_proto::v1::ResourceList;
use ucdk_provider::{CellIdentity, CellName, CellUid, Kind, Node, NodePath, Provider, Stamp};
use ucdk_provider_fake::{Fake, FakeConfig};

use crate::status::{self, CallError};

/// The five verbs, which share one signature on the C side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Validate and lower a resource list; nothing else is touched.
    LoadCatalog,
    /// Observe and plan.
    Plan,
    /// Carry out the plan whose hash the caller pinned.
    Apply,
    /// Carry out the approved destroy plan.
    Destroy,
    /// Read the ledger and the live objects.
    Status,
    /// Compare the cell's stamped objects with the ledger.
    Leaks,
}

/// The cell of a session whose config names none: every stamp it writes carries this identity.
fn local_cell() -> CellIdentity {
    CellIdentity::new(
        CellName::new("local").expect("the literal name is valid"),
        CellName::new("default").expect("the literal name is valid"),
        CellUid::from_text("lllllllllllllllllllllllllm").expect("the literal id is valid"),
    )
}

/// A cell in the text form `CellIdentity` prints: `machine/cell/uid`.
fn parse_cell(text: &str) -> Result<CellIdentity, CallError> {
    let bad = || {
        CallError::new(
            status::INVALID_ARGUMENT,
            format!("cell {text:?} is not machine/cell/uid"),
        )
    };
    let mut parts = text.split('/');
    let (Some(machine), Some(cell), Some(uid), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    match (
        CellName::new(machine),
        CellName::new(cell),
        CellUid::from_text(uid),
    ) {
        (Ok(machine), Ok(cell), Ok(uid)) => Ok(CellIdentity::new(machine, cell, uid)),
        _ => Err(bad()),
    }
}

fn unsupported(what: &str, value: &str) -> CallError {
    CallError::new(status::ABI, format!("this library has no {what} {value:?}"))
}

/// The fake with one object that carries `cell`'s stamp and that no ledger records, as a run
/// that lost its ledger would leave: how a host sees a leak without a cloud.
fn leaky_fake(cell: &CellIdentity) -> Fake {
    let mut fake = Fake::new(FakeConfig::standard());
    let path = NodePath::new("ghost/run").expect("the literal path is valid");
    let name = "ghost-run".to_owned();
    let node = Node::new(path.clone(), Kind::service(), name, Vec::new())
        .expect("a node with no fields has no repeated field");
    let stamp = Stamp::for_node(cell, &path);
    fake.create(&node, &stamp)
        .expect("a fresh fake takes a stamped create");
    fake
}

fn provider_named(name: &str, cell: &CellIdentity) -> Result<Box<dyn Provider>, CallError> {
    match name {
        "fake" => Ok(Box::new(Fake::new(FakeConfig::standard()))),
        "fake-limited" => Ok(Box::new(Fake::new(FakeConfig::limited()))),
        "fake-leaky" => Ok(Box::new(leaky_fake(cell))),
        other => Err(unsupported("provider", other)),
    }
}

/// The clock the ledger times its leases by: the wall clock, or one fixed at `test_clock_ms`.
#[cfg(feature = "test-clock")]
fn clock_for(test_clock_ms: u64) -> Result<Arc<dyn Clock>, CallError> {
    Ok(if test_clock_ms == 0 {
        Arc::new(SystemClock)
    } else {
        Arc::new(ManualClock::at(test_clock_ms))
    })
}

/// The clock the ledger times its leases by: the wall clock. A caller that could fix it would
/// defeat the lease and the fence, so this build refuses `test_clock_ms`.
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

fn ledger_for(
    kind: &str,
    location: &str,
    clock: Arc<dyn Clock>,
) -> Result<Box<dyn Ledger>, CallError> {
    match kind {
        "" | "memory" => Ok(Box::new(MemoryLedger::with_clock(clock))),
        "file" if location.is_empty() => Err(CallError::new(
            status::INVALID_ARGUMENT,
            "a file ledger needs ledger_location",
        )),
        "file" => FileLedger::open_with_clock(location, clock)
            .map(|ledger| Box::new(ledger) as Box<dyn Ledger>)
            .map_err(|e| CallError::new(status::INTERNAL, format!("cannot open the ledger: {e}"))),
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
    provider: Box<dyn Provider>,
    ledger: Box<dyn Ledger>,
    cell: CellIdentity,
    /// The lease holder name; none: the engine's own, new for every run.
    holder: Option<String>,
}

impl Inner {
    fn handle(&mut self, request: Request) -> Outcome {
        let mut engine = Engine::new(
            self.provider.as_mut(),
            self.ledger.as_ref(),
            self.cell.clone(),
        );
        if let Some(holder) = &self.holder {
            engine = engine.with_holder(holder);
        }
        engine.handle(request)
    }

    /// The bytes of the answer to a request message that may be refused before the engine.
    fn answer(&mut self, request: Result<Request, RequestError>) -> Vec<u8> {
        let outcome = match request {
            Ok(request) => v1::Outcome::from(&self.handle(request)),
            Err(refused) => refused.into_outcome(),
        };
        outcome.encode_to_vec()
    }

    /// An apply or a destroy: plan the world as it is now in `mode`, then hand the plan to the
    /// engine with the hash the caller pinned. The engine refuses unless the two hashes agree.
    fn approved(
        &mut self,
        resources: Option<ResourceList>,
        mode: Mode,
        approval: Result<Approved, RequestError>,
        finish: fn(Approved, Plan) -> Request,
    ) -> Vec<u8> {
        let planned = PlanRequest {
            resources,
            mode: mode as i32,
        }
        .to_request();
        let (approval, planned) = match (approval, planned) {
            (Ok(approval), Ok(planned)) => (approval, planned),
            (Err(refused), _) | (_, Err(refused)) => return refused.into_outcome().encode_to_vec(),
        };
        match self.handle(planned) {
            Outcome::Planned(plan) => self.answer(Ok(finish(approval, plan))),
            other => v1::Outcome::from(&other).encode_to_vec(),
        }
    }

    fn run(&mut self, verb: Verb, bytes: &[u8]) -> Result<Vec<u8>, CallError> {
        Ok(match verb {
            Verb::LoadCatalog => {
                let list: ResourceList = decode(bytes, "ResourceList")?;
                catalog_response(lower(&list)).encode_to_vec()
            }
            Verb::Plan => {
                let request: PlanRequest = decode(bytes, "PlanRequest")?;
                self.answer(request.to_request())
            }
            Verb::Status => {
                let request: StatusRequest = decode(bytes, "StatusRequest")?;
                self.answer(request.to_request())
            }
            Verb::Leaks => {
                let request: LeaksRequest = decode(bytes, "LeaksRequest")?;
                self.answer(Ok(request.to_request()))
            }
            Verb::Apply => {
                let request: ApplyRequest = decode(bytes, "ApplyRequest")?;
                let approval = request.approved();
                self.approved(request.resources, Mode::Apply, approval, Approved::apply)
            }
            Verb::Destroy => {
                let request: DestroyRequest = decode(bytes, "DestroyRequest")?;
                let approval = request.approved();
                self.approved(
                    request.resources,
                    Mode::Destroy,
                    approval,
                    Approved::destroy,
                )
            }
        })
    }
}

/// An open session: one provider, one ledger and the cell identity.
///
/// A call holds the session's lock without waiting, so a second call at the same time is refused
/// with `BUSY`. A panic inside a call leaves the lock poisoned, and every later call on that
/// session is refused with `PANIC`: the engine may have stopped between a write-ahead record and
/// the change it announces, and the ledger, not memory, is how a run is picked up again.
pub struct Session {
    inner: Mutex<Inner>,
}

impl Session {
    /// Opens a session from a `SessionConfig` message.
    ///
    /// # Errors
    ///
    /// `DECODE` for bytes that are not a config, `ABI` for a provider, a ledger kind, a provider
    /// option or a credential source this library does not have, `INVALID_ARGUMENT` for a cell
    /// that is not `machine/cell/uid` or a file ledger without a location, and `INTERNAL` when
    /// the ledger file cannot be opened.
    pub fn open(config: &[u8]) -> Result<Self, CallError> {
        let config: SessionConfig = decode(config, "SessionConfig")?;
        if let Some(name) = config.provider_options.keys().next() {
            return Err(unsupported("provider option", name));
        }
        if CredentialSource::try_from(config.credentials).is_err() {
            return Err(unsupported(
                "credential source",
                &config.credentials.to_string(),
            ));
        }
        let cell = if config.cell.is_empty() {
            local_cell()
        } else {
            parse_cell(&config.cell)?
        };
        let session = Self::with_parts(
            provider_named(&config.provider, &cell)?,
            ledger_for(
                &config.ledger_kind,
                &config.ledger_location,
                clock_for(config.test_clock_ms)?,
            )?,
            cell,
        );
        Ok(if config.holder.is_empty() {
            session
        } else {
            session.with_holder(config.holder)
        })
    }

    /// The session, with its runs taking the ledger lease as `holder`.
    #[must_use]
    pub fn with_holder(mut self, holder: impl Into<String>) -> Self {
        self.inner
            .get_mut()
            .expect("the lock is never poisoned")
            .holder = Some(holder.into());
        self
    }

    /// A session over the given parts: how a Rust host, or a test with a provider of its own,
    /// builds one without a config. Hand it to [`crate::into_handle`] to get the C handle.
    pub fn with_parts(
        provider: Box<dyn Provider>,
        ledger: Box<dyn Ledger>,
        cell: CellIdentity,
    ) -> Self {
        Self {
            inner: Mutex::new(Inner {
                provider,
                ledger,
                cell,
                holder: None,
            }),
        }
    }

    /// Runs one verb over the request bytes and returns the response bytes.
    ///
    /// # Errors
    ///
    /// `BUSY` when another call holds the session, `PANIC` when an earlier call panicked, and
    /// `DECODE` when `request` is not the verb's message. A plan the engine refuses is not an
    /// error: it is a response.
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
