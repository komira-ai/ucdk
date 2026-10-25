// SPDX-License-Identifier: Apache-2.0
#![allow(dead_code)]
//! Helpers shared by the engine's tests: resource lists, a driver that plans and then applies,
//! a provider that runs code before a chosen call, and a ledger that can be made to fail.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use ucdk_engine::{Engine, Mode, Outcome, Report, Request};
use ucdk_ledger::{Change, FileLedger, Ledger, LedgerError, MemoryLedger, Record, Snapshot, Token};
use ucdk_plan::{Lowered, Plan, lower};
use ucdk_proto::v1::resource::Body;
use ucdk_proto::v1::{Access, Image, Ref, Resource, ResourceList, Service, Uses, image, service};
use ucdk_provider::{
    CellIdentity, CellScope, Created, Deletability, Kind, Live, Node, NodePath, Observation,
    OwnedObject, OwnerScope, PhysicalId, Provider, ProviderError, StampMode, StateClass, Updated,
    kit,
};
use ucdk_provider_fake::Fake;

/// The cell the engine tests run in.
pub fn cell() -> CellIdentity {
    kit::cell()
}

// ---- resource lists --------------------------------------------------------------------------

/// A service with the given port; `public` turns its public role on.
pub fn service_of(id: &str, port: u32, public: bool, uses: &[&str]) -> Resource {
    let exposure = public.then_some(service::Exposure::Public(service::Public {}));
    Resource {
        id: id.to_owned(),
        body: Some(Body::Service(Service {
            image: Some(Image {
                source: Some(image::Source::Digest("sha256:aa".to_owned())),
                platform: String::new(),
            }),
            port,
            exposure,
            ..Service::default()
        })),
        uses: uses
            .iter()
            .map(|target| Uses {
                target: Some(Ref {
                    resource: (*target).to_owned(),
                    output: None,
                }),
                access: Access::Call as i32,
            })
            .collect(),
    }
}

pub fn lowered(resources: Vec<Resource>) -> Lowered {
    lower(&ResourceList {
        resource: resources,
    })
    .expect("the list lowers")
}

/// `web` (public) calls `api`, which calls `db`: four wanted nodes, `db/run`, `api/run`,
/// `web/run` and `web/public`.
pub fn chain() -> Lowered {
    chain_with(5432, true)
}

pub fn chain_with(db_port: u32, web_public: bool) -> Lowered {
    lowered(vec![
        service_of("web", 80, web_public, &["api"]),
        service_of("api", 81, false, &["db"]),
        service_of("db", db_port, false, &[]),
    ])
}

pub fn path(text: &str) -> NodePath {
    NodePath::new(text).expect("a valid path")
}

pub fn paths(list: &[&str]) -> Vec<NodePath> {
    list.iter().map(|p| path(p)).collect()
}

// ---- drivers ---------------------------------------------------------------------------------

pub fn plan_of(provider: &mut dyn Provider, ledger: &dyn Ledger, desired: &Lowered) -> Outcome {
    Engine::new(provider, ledger, cell()).handle(Request::Plan {
        desired: desired.clone(),
        mode: Mode::Apply,
    })
}

/// Plans, then applies the plan under its own hash, as an approver would.
pub fn apply_with(
    provider: &mut dyn Provider,
    ledger: &dyn Ledger,
    desired: &Lowered,
    rollback: bool,
) -> Outcome {
    let plan = match plan_of(provider, ledger, desired) {
        Outcome::Planned(plan) => plan,
        other => return other,
    };
    let plan_hash = plan.hash().clone();
    Engine::new(provider, ledger, cell()).handle(Request::Apply {
        desired: desired.clone(),
        plan,
        plan_hash,
        rollback,
    })
}

pub fn apply(provider: &mut dyn Provider, ledger: &dyn Ledger, desired: &Lowered) -> Outcome {
    apply_with(provider, ledger, desired, false)
}

/// Plans a destroy of `desired`, then runs it under that plan's hash.
pub fn destroy(provider: &mut dyn Provider, ledger: &dyn Ledger, desired: &Lowered) -> Outcome {
    let plan = match Engine::new(provider, ledger, cell()).handle(Request::Plan {
        desired: desired.clone(),
        mode: Mode::Destroy,
    }) {
        Outcome::Planned(plan) => plan,
        other => return other,
    };
    let plan_hash = plan.hash().clone();
    Engine::new(provider, ledger, cell()).handle(Request::Destroy {
        desired: desired.clone(),
        plan,
        plan_hash,
    })
}

/// The report of a run that finished.
pub fn done(outcome: Outcome) -> Report {
    match outcome {
        Outcome::Done(report) => report,
        other => panic!("the run did not finish: {other:?}"),
    }
}

pub fn planned(outcome: Outcome) -> Plan {
    match outcome {
        Outcome::Planned(plan) => plan,
        other => panic!("not a plan: {other:?}"),
    }
}

// ---- looking at the world --------------------------------------------------------------------

/// The mutating calls the fake has logged so far.
pub fn mutations(fake: &Fake) -> usize {
    fake.calls().iter().filter(|c| c.op.mutates()).count()
}

/// The objects of the fake, one per line (ids, names, labels, fields), without the call log.
pub fn objects(fake: &Fake) -> String {
    let dump = fake.dump();
    let end = dump.find("calls:").expect("the dump has a call section");
    dump[..end].to_owned()
}

pub fn records(ledger: &dyn Ledger) -> BTreeMap<NodePath, Record> {
    ledger.read().expect("read").records
}

pub fn physical_of(ledger: &dyn Ledger, p: &str) -> PhysicalId {
    records(ledger)[&path(p)]
        .physical
        .clone()
        .expect("the record names an object")
}

/// Every object of every owner the ledger records, by physical id.
pub fn owned_by_fake(fake: &mut Fake, owners: &[&str]) -> Vec<PhysicalId> {
    let mut found = Vec::new();
    for owner in owners {
        let scope = OwnerScope::new(
            cell(),
            ucdk_provider::OwnerLabel::new(owner).expect("owner"),
        );
        found.extend(
            fake.list_owned(&scope)
                .expect("list")
                .into_iter()
                .map(|o: OwnedObject| o.physical),
        );
    }
    found.sort();
    found
}

// ---- a provider that runs code before a chosen call ------------------------------------------

type Hook<'f> = Box<dyn FnOnce(&mut Fake) + Send + 'f>;

/// The fake, with code run just before the `at`th call that reads or mutates (counted from 1).
pub struct Hooked<'f> {
    fake: &'f mut Fake,
    calls: usize,
    at: usize,
    hook: Option<Hook<'f>>,
    silent: bool,
}

impl<'f> Hooked<'f> {
    pub fn new(fake: &'f mut Fake, at: usize, hook: impl FnOnce(&mut Fake) + Send + 'f) -> Self {
        Self {
            fake,
            calls: 0,
            at,
            hook: Some(Box::new(hook)),
            silent: false,
        }
    }

    /// The fake as a provider that reports no outputs for any object.
    pub fn silent(fake: &'f mut Fake) -> Self {
        Self {
            silent: true,
            ..Self::new(fake, usize::MAX, |_| {})
        }
    }

    /// How many reading and mutating calls have been made.
    pub fn calls(&self) -> usize {
        self.calls
    }

    fn tick(&mut self) {
        self.calls += 1;
        if self.calls == self.at
            && let Some(hook) = self.hook.take()
        {
            hook(self.fake);
        }
    }
}

impl Provider for Hooked<'_> {
    fn state_class(&self, kind: &Kind) -> StateClass {
        self.fake.state_class(kind)
    }

    fn deletability(&self, kind: &Kind, live: &Live) -> Deletability {
        self.fake.deletability(kind, live)
    }

    fn stamp_mode(&self, kind: &Kind) -> StampMode {
        self.fake.stamp_mode(kind)
    }

    fn observe(
        &mut self,
        node: &Node,
        hint: Option<&PhysicalId>,
    ) -> Result<Observation, ProviderError> {
        self.tick();
        let mut seen = self.fake.observe(node, hint)?;
        if let (true, Observation::Present(live)) = (self.silent, &mut seen) {
            live.outputs.clear();
        }
        Ok(seen)
    }

    fn create(
        &mut self,
        node: &Node,
        stamp: &ucdk_provider::Stamp,
    ) -> Result<Created, ProviderError> {
        self.tick();
        self.fake.create(node, stamp)
    }

    fn update(&mut self, node: &Node, live: &Live) -> Result<Updated, ProviderError> {
        self.tick();
        let mut updated = self.fake.update(node, live)?;
        if self.silent {
            updated.outputs.clear();
        }
        Ok(updated)
    }

    fn delete(&mut self, kind: &Kind, physical: &PhysicalId) -> Result<(), ProviderError> {
        self.tick();
        self.fake.delete(kind, physical)
    }

    fn list_owned(&mut self, scope: &OwnerScope) -> Result<Vec<OwnedObject>, ProviderError> {
        self.tick();
        self.fake.list_owned(scope)
    }

    fn list_stamped(&mut self, scope: &CellScope) -> Result<Vec<OwnedObject>, ProviderError> {
        self.tick();
        self.fake.list_stamped(scope)
    }
}

// ---- the ledgers the scenarios run on ---------------------------------------------------------

/// The ledger implementations the shared scenarios run against.
#[derive(Debug, Clone, Copy)]
pub enum Backend {
    Memory,
    File,
}

impl Backend {
    /// A new, empty ledger of this kind.
    pub fn ledger(self) -> AnyLedger {
        match self {
            Self::Memory => AnyLedger::Memory(MemoryLedger::new()),
            Self::File => {
                let dir = Scratch::new();
                let ledger =
                    FileLedger::open(dir.0.join("ledger.ucdk")).expect("open the file ledger");
                AnyLedger::File(ledger, Arc::new(dir))
            }
        }
    }
}

/// A directory for one file ledger, removed when the last handle to the ledger is dropped.
#[derive(Debug)]
pub struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("engine-ledgers-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("make the scratch directory");
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A ledger of either kind. A clone shares the document, as a clone of each kind does.
#[derive(Debug, Clone)]
pub enum AnyLedger {
    Memory(MemoryLedger),
    File(FileLedger, Arc<Scratch>),
}

impl Ledger for AnyLedger {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        match self {
            Self::Memory(ledger) => ledger.read(),
            Self::File(ledger, _) => ledger.read(),
        }
    }

    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError> {
        match self {
            Self::Memory(ledger) => ledger.write(expected, change),
            Self::File(ledger, _) => ledger.write(expected, change),
        }
    }
}

// ---- a ledger that can be made to fail -------------------------------------------------------

/// A memory ledger that counts writes and can fail the `n`th write, fail every read, or report
/// every write as stale.
#[derive(Debug)]
pub struct FlakyLedger {
    pub inner: AnyLedger,
    writes: AtomicU64,
    fail_write_at: AtomicU64,
    fail_reads: AtomicBool,
    always_stale: AtomicBool,
}

impl FlakyLedger {
    pub fn new() -> Self {
        Self::on(Backend::Memory)
    }

    pub fn on(backend: Backend) -> Self {
        Self {
            inner: backend.ledger(),
            writes: AtomicU64::default(),
            fail_write_at: AtomicU64::default(),
            fail_reads: AtomicBool::default(),
            always_stale: AtomicBool::default(),
        }
    }

    pub fn writes(&self) -> u64 {
        self.writes.load(Ordering::SeqCst)
    }

    /// Fails the `n`th write from the start (1-based); 0 clears it.
    pub fn fail_write_at(&self, n: u64) {
        self.fail_write_at.store(n, Ordering::SeqCst);
    }

    pub fn fail_reads(&self, on: bool) {
        self.fail_reads.store(on, Ordering::SeqCst);
    }

    pub fn always_stale(&self, on: bool) {
        self.always_stale.store(on, Ordering::SeqCst);
    }
}

impl Ledger for FlakyLedger {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        if self.fail_reads.load(Ordering::SeqCst) {
            return Err(LedgerError::Unavailable("reads are down".to_owned()));
        }
        self.inner.read()
    }

    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError> {
        let n = self.writes.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_write_at.load(Ordering::SeqCst) == n {
            return Err(LedgerError::Unavailable("the write was lost".to_owned()));
        }
        if self.always_stale.load(Ordering::SeqCst) {
            return Err(LedgerError::Stale);
        }
        self.inner.write(expected, change)
    }
}

// ---- value references ------------------------------------------------------------------------

/// A service `id` whose env var `env_key` takes the `output` of the resource `producer`; it
/// also calls `producer`, as a service that reads another's address does.
pub fn service_reading(
    id: &str,
    port: u32,
    env_key: &str,
    producer: &str,
    output: ucdk_proto::v1::Output,
) -> Resource {
    let mut resource = service_of(id, port, false, &[producer]);
    let Some(Body::Service(service)) = &mut resource.body else {
        unreachable!("service_of builds a service");
    };
    service.env.insert(
        env_key.to_owned(),
        ucdk_proto::v1::Value {
            v: Some(ucdk_proto::v1::value::V::Ref(Ref {
                resource: producer.to_owned(),
                output: Some(ucdk_proto::v1::r#ref::Output::Standard(output as i32)),
            })),
        },
    );
    resource
}

/// `app` reads the URL of `db`: two services, `db/run` first.
pub fn bound_pair() -> Lowered {
    lowered(vec![
        service_reading("app", 80, "DB_URL", "db", ucdk_proto::v1::Output::Url),
        service_of("db", 5432, false, &[]),
    ])
}
