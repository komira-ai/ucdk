// SPDX-License-Identifier: Apache-2.0
//! The rig the engine's tests run on: the project's first machine over the fakes, a ledger
//! that can be made to fail, and the few wrappers that let a test do something to the world
//! while the engine is in the middle of a call. Time is a `ManualClock` shared by the ledger
//! and the fakes; nothing here waits for anything.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};
use ucdk_core::{Clock, Hash256, ManualClock, RecordKey};
use ucdk_ledger::{
    Change, Fence, Lease, LeaseError, Ledger, LedgerError, MemoryLedger, Origin, OwnerPath,
    Record as Row, Snapshot, State, Token,
};
use ucdk_release::codec;
use ucdk_release::machine::Checked;
use ucdk_release::publish::{Held, Presence};
use ucdk_release::selection::Selection;
use ucdk_release::set_document;
use ucdk_release::store::{SetScope, StoreKey};
use ucdk_release_engine::{Engine, Outcome, Request, Run, SET_FILE, Seams};
use ucdk_release_fake::{FakeBuildSystem, FakePause, FakeRegistry, FakeStore};
use ucdk_release_proto::v1::action::Kind as ActionKind;
use ucdk_release_proto::v1::manifest::Metadata;
use ucdk_release_proto::v1::push_identity::Kind as PushKind;
use ucdk_release_proto::v1::{
    Action, Anonymous, ArtifactDecl, ArtifactType, Build, BuildStamp, BuildSystem, BuildSystemKind,
    Channel, ChannelKind, Coordinate, FileMeta, Machine, Manifest, Member, Producer, Publish,
    PushIdentity, ReleaseSet, Repository, Smoke, Stage, Stamped, Target, Test, Trigger, Validation,
    ValidationKind, Visibility,
};
use ucdk_release_seams::{
    ArtifactStore, Blob, BuildSystem as BuildSystemSeam, CommitAnswer, CommitUnit, EntryWriter,
    Grant, Heartbeat, Invocation, ListScope, Listed, Located, Publisher, SeamError, StageAnswer,
    StageItem, TestOutcome, YankUnit,
};

pub const PLATFORM: &str = "linux-x86_64";
pub const REVISION: &str = "0123456789abcdef0123456789abcdef01234567";
pub const START: u64 = 1_000_000;
pub const TTL: Duration = Duration::from_secs(600);
pub const GENERATION: &str = "r1-a1";

pub fn stamp() -> BuildStamp {
    BuildStamp {
        version: "1.2.3".to_owned(),
        build_number: 41,
        source_commit: "abc123".to_owned(),
        timestamp_ms: 1_700_000_000_000,
    }
}

// ---- the machine ----------------------------------------------------------------------------

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

pub fn target(label: &str) -> Target {
    Target {
        label: label.to_owned(),
        flags: Vec::new(),
        env: BTreeMap::new(),
    }
}

fn artifact(name: &str, label: &str) -> ArtifactDecl {
    ArtifactDecl {
        name: name.to_owned(),
        r#type: ArtifactType::File as i32,
        needs: Vec::new(),
        target: Some(Target {
            flags: strings(&["-c", "release.version={release_version}"]),
            ..target(label)
        }),
        labels: BTreeMap::new(),
    }
}

pub fn validation(name: &str, kind: ValidationKind, wait_polls: u32) -> Validation {
    Validation {
        name: name.to_owned(),
        kind: kind as i32,
        wait_polls,
    }
}

pub fn test_action(name: &str, labels: &[&str]) -> Action {
    Action {
        name: name.to_owned(),
        kind: Some(ActionKind::Test(Test {
            targets: labels.iter().map(|l| target(l)).collect(),
        })),
        validations: Vec::new(),
    }
}

fn stage(name: &str, needs: &[&str], actions: Vec<Action>) -> Stage {
    Stage {
        name: name.to_owned(),
        needs: strings(needs),
        trigger: Trigger::Release as i32,
        environment: String::new(),
        exceptional: false,
        actions,
        validations: Vec::new(),
    }
}

/// The first machine of the project, as the design writes it, with a Test-only stage `verify`
/// of two actions added. `build` builds and seals and tests; `publish` pushes to a public
/// registry and reads the packages back.
pub fn machine() -> Machine {
    let repository = Repository {
        r#type: ArtifactType::File as i32,
        location: "https://packages.example.com/files".to_owned(),
        visibility: Visibility::Public as i32,
        push: Some(PushIdentity {
            kind: Some(PushKind::Anonymous(Anonymous {})),
        }),
    };
    Machine {
        schema_version: 1,
        name: "ucdk".to_owned(),
        stages: vec![
            stage(
                "build",
                &[],
                vec![
                    Action {
                        name: "build".to_owned(),
                        kind: Some(ActionKind::Build(Build {
                            platform: PLATFORM.to_owned(),
                            artifacts: strings(&["ucdk-cli", "ucdk-lib"]),
                        })),
                        validations: vec![validation("closure", ValidationKind::SetClosure, 0)],
                    },
                    test_action("test", &["//release:set_test"]),
                ],
            ),
            Stage {
                environment: "publish".to_owned(),
                ..stage(
                    "publish",
                    &["build"],
                    vec![
                        Action {
                            name: "publish".to_owned(),
                            kind: Some(ActionKind::Publish(Publish {
                                platform: PLATFORM.to_owned(),
                                artifacts: strings(&["ucdk-cli", "ucdk-lib"]),
                                channel: "registry".to_owned(),
                                attempts: 0,
                                polls: 0,
                            })),
                            validations: Vec::new(),
                        },
                        Action {
                            name: "smoke".to_owned(),
                            kind: Some(ActionKind::Smoke(Smoke {
                                platform: PLATFORM.to_owned(),
                                artifacts: strings(&["ucdk-cli", "ucdk-lib"]),
                                channel: "registry".to_owned(),
                            })),
                            validations: vec![validation(
                                "visible",
                                ValidationKind::ChannelReadback,
                                2,
                            )],
                        },
                    ],
                )
            },
            stage(
                "verify",
                &[],
                vec![
                    test_action("unit", &["//release:unit"]),
                    test_action("lint", &["//release:lint"]),
                ],
            ),
        ],
        artifacts: vec![
            artifact("ucdk-cli", "//release:ucdk_cli_package"),
            artifact("ucdk-lib", "//release:ucdk_lib_package"),
        ],
        channels: vec![Channel {
            name: "registry".to_owned(),
            repositories: vec![repository],
            monotonic: false,
            kind: ChannelKind::PackageRegistry as i32,
        }],
        stamp_ignore: Vec::new(),
        build_system: Some(BuildSystem {
            kind: BuildSystemKind::Buck2 as i32,
            binary: "buck2".to_owned(),
            config_file: "release/release.buckconfig".to_owned(),
            mode: "release".to_owned(),
            flags: Vec::new(),
            env: BTreeMap::from([("PATH".to_owned(), "/opt/tools/bin:/usr/bin:/bin".to_owned())]),
        }),
    }
}

pub fn stage_mut<'a>(m: &'a mut Machine, name: &str) -> &'a mut Stage {
    m.stages
        .iter_mut()
        .find(|s| s.name == name)
        .expect("the stage exists")
}

pub fn action_mut<'a>(m: &'a mut Machine, stage: &str, action: &str) -> &'a mut Action {
    stage_mut(m, stage)
        .actions
        .iter_mut()
        .find(|a| a.name == action)
        .expect("the action exists")
}

/// The targets of a Test action, replaced by `labels`.
pub fn set_targets(m: &mut Machine, stage: &str, action: &str, labels: &[&str]) {
    match &mut action_mut(m, stage, action).kind {
        Some(ActionKind::Test(t)) => t.targets = labels.iter().map(|l| target(l)).collect(),
        _ => panic!("{action} is a Test action"),
    }
}

// ---- scratch ----------------------------------------------------------------------------------

/// A directory under the build's scratch area, empty on creation and removed on drop.
#[derive(Debug)]
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("engine-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("make the scratch directory");
        Self(dir)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The names in the directory, sorted.
    pub fn entries(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(&self.0)
            .expect("read the scratch directory")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

// ---- a ledger that can be made to fail ------------------------------------------------------------

/// A memory ledger on the shared clock that counts every call and can fail calls: reads, the
/// answer of a grant, releases, or every call from the `n`th on (a process that died there).
#[derive(Debug, Clone)]
pub struct Chaos {
    pub inner: MemoryLedger,
    state: Arc<ChaosState>,
}

#[derive(Debug, Default)]
struct ChaosState {
    calls: AtomicU64,
    crash_at: AtomicU64,
    fail_reads: AtomicBool,
    lose_acquire_answer: AtomicBool,
    fail_release: AtomicBool,
    acquire_ifs: AtomicU64,
    renews: AtomicU64,
    fail_writes: AtomicBool,
}

impl Chaos {
    pub fn new(clock: &ManualClock) -> Self {
        Self {
            inner: MemoryLedger::with_clock(Arc::new(clock.clone())),
            state: Arc::default(),
        }
    }

    /// Every call from the `n`th (1 is the next) fails as unavailable until `revive`.
    pub fn crash_at(&self, n: u64) {
        self.state
            .crash_at
            .store(self.calls() + n, Ordering::SeqCst);
    }

    pub fn revive(&self) {
        self.state.crash_at.store(0, Ordering::SeqCst);
    }

    pub fn calls(&self) -> u64 {
        self.state.calls.load(Ordering::SeqCst)
    }

    pub fn fail_reads(&self, on: bool) {
        self.state.fail_reads.store(on, Ordering::SeqCst);
    }

    pub fn fail_writes(&self, on: bool) {
        self.state.fail_writes.store(on, Ordering::SeqCst);
    }

    pub fn lose_acquire_answer(&self, on: bool) {
        self.state.lose_acquire_answer.store(on, Ordering::SeqCst);
    }

    pub fn fail_release(&self, on: bool) {
        self.state.fail_release.store(on, Ordering::SeqCst);
    }

    /// The renewals that reached the ledger.
    pub fn renews(&self) -> u64 {
        self.state.renews.load(Ordering::SeqCst)
    }

    /// The conditional grants that reached the ledger.
    pub fn acquire_ifs(&self) -> u64 {
        self.state.acquire_ifs.load(Ordering::SeqCst)
    }

    fn gate(&self) -> Result<(), String> {
        let n = self.state.calls.fetch_add(1, Ordering::SeqCst) + 1;
        let crash = self.state.crash_at.load(Ordering::SeqCst);
        if crash != 0 && n >= crash {
            return Err("the process died here".to_owned());
        }
        Ok(())
    }

    /// The records of the ledger, by path.
    pub fn records(&self) -> BTreeMap<String, Row> {
        self.inner
            .read()
            .expect("read")
            .records
            .into_iter()
            .map(|(path, row)| (path.to_string(), row))
            .collect()
    }

    /// The decoded release records with their states.
    pub fn decoded(&self) -> Vec<(codec::Record, State)> {
        self.inner
            .read()
            .expect("read")
            .records
            .values()
            .map(|row| {
                let encoded = codec::Encoded {
                    kind: row.kind.clone(),
                    name: row.name.clone(),
                    physical: row.physical.clone(),
                    outputs: row.outputs.clone(),
                };
                (codec::Record::decode(&encoded).expect("decodes"), row.state)
            })
            .collect()
    }

    /// The action record of `stage`/`action`.
    pub fn action(&self, stage: &str, action: &str) -> Option<(codec::Action, State)> {
        self.decoded().into_iter().find_map(|(r, s)| match r {
            codec::Record::Action(a) if a.stage == stage && a.action == action => Some((a, s)),
            _ => None,
        })
    }

    pub fn validation(&self, stage: &str, name: &str) -> Option<(codec::Validation, State)> {
        self.decoded().into_iter().find_map(|(r, s)| match r {
            codec::Record::Validation(v) if v.stage == stage && v.validation == name => {
                Some((v, s))
            }
            _ => None,
        })
    }

    pub fn run_record(&self) -> Option<(codec::Run, State)> {
        self.decoded().into_iter().find_map(|(r, s)| match r {
            codec::Record::Run(run) => Some((run, s)),
            _ => None,
        })
    }

    /// Writes `record` in `state` without a lease, as a record that was already there.
    pub fn seed(&self, record: &codec::Record, state: State) {
        let encoded = record.encode().expect("encodes");
        let path = encoded.path().expect("a path");
        let row = Row {
            owner: OwnerPath::of(&path),
            path,
            kind: encoded.kind,
            name: encoded.name,
            origin: Origin::Created,
            state,
            physical: None,
            deps: Default::default(),
            outputs: encoded.outputs,
        };
        let token = self.inner.read().expect("read").token;
        self.inner.write(&token, Change::Put(row)).expect("seed");
    }

    /// The lease on the ledger: who holds it and until when.
    pub fn lease(&self) -> Option<(String, u64)> {
        self.inner
            .read()
            .expect("read")
            .lease
            .map(|h| (h.holder, h.expires_at))
    }
}

impl Ledger for Chaos {
    fn read(&self) -> Result<Snapshot, LedgerError> {
        self.gate().map_err(LedgerError::Unavailable)?;
        if self.state.fail_reads.load(Ordering::SeqCst) {
            return Err(LedgerError::Unavailable("reads are down".to_owned()));
        }
        self.inner.read()
    }

    fn write(&self, expected: &Token, change: Change) -> Result<Token, LedgerError> {
        self.gate().map_err(LedgerError::Unavailable)?;
        self.inner.write(expected, change)
    }

    fn write_fenced(
        &self,
        fence: Fence,
        expected: &Token,
        change: Change,
    ) -> Result<Token, LedgerError> {
        self.gate().map_err(LedgerError::Unavailable)?;
        if self.state.fail_writes.load(Ordering::SeqCst) {
            return Err(LedgerError::Unavailable("writes are down".to_owned()));
        }
        self.inner.write_fenced(fence, expected, change)
    }

    fn acquire(&self, holder: &str, ttl: Duration) -> Result<Lease, LeaseError> {
        self.gate().map_err(LeaseError::Unavailable)?;
        let lease = self.inner.acquire(holder, ttl)?;
        if self.state.lose_acquire_answer.load(Ordering::SeqCst) {
            return Err(LeaseError::Unavailable("the answer was lost".to_owned()));
        }
        Ok(lease)
    }

    fn acquire_if(
        &self,
        holder: &str,
        ttl: Duration,
        token: &Token,
        fence: Fence,
    ) -> Result<Lease, LeaseError> {
        self.gate().map_err(LeaseError::Unavailable)?;
        self.state.acquire_ifs.fetch_add(1, Ordering::SeqCst);
        self.inner.acquire_if(holder, ttl, token, fence)
    }

    fn renew(&self, fence: Fence, ttl: Duration) -> Result<Lease, LedgerError> {
        self.gate().map_err(LedgerError::Unavailable)?;
        self.state.renews.fetch_add(1, Ordering::SeqCst);
        self.inner.renew(fence, ttl)
    }

    fn release(&self, fence: Fence) -> Result<(), LedgerError> {
        self.gate().map_err(LedgerError::Unavailable)?;
        if self.state.fail_release.load(Ordering::SeqCst) {
            return Err(LedgerError::Unavailable("the release was lost".to_owned()));
        }
        self.inner.release(fence)
    }
}

// ---- seams that can be made to say something else -------------------------------------------------

/// A store whose reads can be made to fail.
#[derive(Default)]
pub struct TestStore {
    pub inner: FakeStore,
    pub get: Option<SeamError>,
}

impl ArtifactStore for TestStore {
    fn begin(&mut self, key: &StoreKey) -> Result<Box<dyn EntryWriter + '_>, SeamError> {
        self.inner.begin(key)
    }

    fn files(&mut self, key: &StoreKey) -> Result<Vec<String>, SeamError> {
        self.inner.files(key)
    }

    fn get(&mut self, key: &StoreKey, rel: &str) -> Result<Option<Vec<u8>>, SeamError> {
        match &self.get {
            Some(error) => Err(error.clone()),
            None => self.inner.get(key, rel),
        }
    }

    fn discard(&mut self, key: &StoreKey) -> Result<(), SeamError> {
        self.inner.discard(key)
    }

    fn keys(&mut self) -> Result<Vec<StoreKey>, SeamError> {
        self.inner.keys()
    }
}

/// What a read of a coordinate should answer instead of the registry's own.
#[derive(Clone)]
pub enum Fetch {
    Real,
    Answer(Result<Option<Vec<u8>>, SeamError>),
}

/// A registry that records whether each read carried a grant, and can answer a fetch with
/// something the registry would not: a registry that lies about what it serves.
pub struct Probe {
    pub inner: FakeRegistry,
    pub fetch: Fetch,
    /// For each observe and fetch, whether the grant it carried was none.
    pub anonymous: Vec<bool>,
    pub observes: u64,
    pub fetches: u64,
    /// Done once, at the first observe: something happens to the world while the engine reads.
    pub on_observe: Option<Box<dyn FnOnce() + Send>>,
}

impl Probe {
    pub fn new(inner: FakeRegistry) -> Self {
        Self {
            inner,
            fetch: Fetch::Real,
            anonymous: Vec::new(),
            observes: 0,
            fetches: 0,
            on_observe: None,
        }
    }
}

impl Publisher for Probe {
    fn types(&self) -> &[ArtifactType] {
        self.inner.types()
    }

    fn stages(&self) -> bool {
        self.inner.stages()
    }

    fn commit_unit(&self) -> CommitUnit {
        self.inner.commit_unit()
    }

    fn conditional_put(&self) -> bool {
        self.inner.conditional_put()
    }

    fn grant_limit(&self) -> usize {
        self.inner.grant_limit()
    }

    fn yank_unit(&self) -> YankUnit {
        self.inner.yank_unit()
    }

    fn observe(
        &mut self,
        g: &Grant,
        k: &Coordinate,
        set: &[StageItem<'_>],
    ) -> Result<Presence, SeamError> {
        if let Some(hook) = self.on_observe.take() {
            hook();
        }
        self.observes += 1;
        self.anonymous.push(g.is_none());
        self.inner.observe(g, k, set)
    }

    fn stage(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<StageAnswer, SeamError> {
        self.inner.stage(g, items)
    }

    fn abort(&mut self, g: &Grant, ks: &[Coordinate]) -> Result<(), SeamError> {
        self.inner.abort(g, ks)
    }

    fn commit(&mut self, g: &Grant, items: &[StageItem<'_>]) -> Result<CommitAnswer, SeamError> {
        self.inner.commit(g, items)
    }

    fn yank(&mut self, g: &Grant, k: &Coordinate, reason: &str) -> Result<(), SeamError> {
        self.inner.yank(g, k, reason)
    }

    fn fetch(&mut self, g: &Grant, k: &Coordinate) -> Result<Option<Vec<u8>>, SeamError> {
        self.fetches += 1;
        self.anonymous.push(g.is_none());
        match &self.fetch {
            Fetch::Real => self.inner.fetch(g, k),
            Fetch::Answer(answer) => answer.clone(),
        }
    }

    fn newest(
        &mut self,
        g: &Grant,
        ch: &str,
        name: &str,
    ) -> Result<Vec<(Coordinate, Held)>, SeamError> {
        self.inner.newest(g, ch, name)
    }

    fn list(&mut self, g: &Grant, scope: &ListScope) -> Result<Vec<Listed>, SeamError> {
        self.inner.list(g, scope)
    }
}

/// A build system that does something just before each test invocation, and records the report
/// path it was given and whether the directory of it existed.
pub struct Watched<'a> {
    pub inner: &'a mut FakeBuildSystem,
    pub before_test: Box<dyn FnMut() + Send + 'a>,
    pub reports: Vec<(String, bool)>,
}

impl<'a> Watched<'a> {
    pub fn new(inner: &'a mut FakeBuildSystem, before_test: impl FnMut() + Send + 'a) -> Self {
        Self {
            inner,
            before_test: Box::new(before_test),
            reports: Vec::new(),
        }
    }
}

impl BuildSystemSeam for Watched<'_> {
    fn kind(&self) -> BuildSystemKind {
        self.inner.kind()
    }

    fn build(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<Vec<Located>, SeamError> {
        self.inner.build(inv, heartbeat)
    }

    fn test(
        &mut self,
        inv: &Invocation<'_>,
        heartbeat: &dyn Heartbeat,
    ) -> Result<TestOutcome, SeamError> {
        let report = inv.report.expect("a test invocation has a report path");
        let directory = Path::new(report).parent().expect("a directory").is_dir();
        self.reports.push((report.to_owned(), directory));
        (self.before_test)();
        self.inner.test(inv, heartbeat)
    }

    fn copy_out(&mut self, out: &Located, into: &mut dyn EntryWriter) -> Result<(), SeamError> {
        self.inner.copy_out(out, into)
    }
}

/// A build system that answers every test with one outcome.
pub struct Scripted(pub TestOutcome);

impl BuildSystemSeam for Scripted {
    fn kind(&self) -> BuildSystemKind {
        BuildSystemKind::Buck2
    }

    fn build(&mut self, _: &Invocation<'_>, _: &dyn Heartbeat) -> Result<Vec<Located>, SeamError> {
        Err(SeamError::Permanent(
            "a scripted build system builds nothing".to_owned(),
        ))
    }

    fn test(&mut self, _: &Invocation<'_>, _: &dyn Heartbeat) -> Result<TestOutcome, SeamError> {
        Ok(self.0.clone())
    }

    fn copy_out(&mut self, _: &Located, _: &mut dyn EntryWriter) -> Result<(), SeamError> {
        Err(SeamError::Permanent(
            "a scripted build system copies nothing".to_owned(),
        ))
    }
}

// ---- the rig --------------------------------------------------------------------------------------

pub struct Rig {
    pub clock: ManualClock,
    pub ledger: Chaos,
    pub machine: Checked,
    pub build: FakeBuildSystem,
    pub store: TestStore,
    pub registry: Probe,
    pub pause: FakePause,
    pub scratch: Scratch,
    pub stamp: BuildStamp,
    pub attempt: u32,
    pub run_id: String,
    pub revision: String,
    pub ttl: Duration,
    /// The directory the engine makes its report directories in: the scratch directory, unless
    /// a test points it elsewhere.
    pub scratch_dir: PathBuf,
    /// Whether the engine is given the publisher of the channel `registry`.
    pub with_publisher: bool,
}

impl Rig {
    pub fn new(name: &str) -> Self {
        Self::with(name, |_| {})
    }

    /// The first machine after `edit`, over fakes that share one clock at `START`.
    pub fn with(name: &str, edit: impl FnOnce(&mut Machine)) -> Self {
        let mut machine = machine();
        edit(&mut machine);
        let clock = ManualClock::at(START);
        let registry = FakeRegistry::new(&clock, "registry")
            .open()
            .without_staging();
        let scratch = Scratch::new(name);
        Self {
            ledger: Chaos::new(&clock),
            machine: Checked::read(machine).expect("the machine passes every rule"),
            build: FakeBuildSystem::new(BuildSystemKind::Buck2, &clock),
            store: TestStore::default(),
            registry: Probe::new(registry),
            pause: FakePause::new(),
            scratch_dir: scratch.path().to_owned(),
            scratch,
            stamp: stamp(),
            attempt: 1,
            run_id: "r1".to_owned(),
            revision: REVISION.to_owned(),
            ttl: TTL,
            with_publisher: true,
            clock,
        }
    }

    /// The ledger's clock, in milliseconds.
    pub fn clock_ms(&self) -> u64 {
        self.clock.now_millis()
    }

    /// Runs `only` of `stage` (all of it when `only` is empty).
    pub fn handle(&mut self, stage: &str, only: &[&str]) -> Outcome {
        let Self {
            ledger,
            machine,
            build,
            store,
            registry,
            pause,
            scratch_dir,
            stamp,
            attempt,
            run_id,
            revision,
            ttl,
            with_publisher,
            ..
        } = self;
        run_on(
            machine,
            ledger,
            build,
            store,
            registry,
            pause,
            scratch_dir,
            stamp,
            *attempt,
            run_id,
            revision,
            *ttl,
            *with_publisher,
            stage,
            only,
        )
    }

    /// Like `handle`, with `build` as the build system.
    pub fn handle_with(
        &mut self,
        build: &mut dyn BuildSystemSeam,
        stage: &str,
        only: &[&str],
    ) -> Outcome {
        let Self {
            ledger,
            machine,
            store,
            registry,
            pause,
            scratch_dir,
            stamp,
            attempt,
            run_id,
            revision,
            ttl,
            with_publisher,
            ..
        } = self;
        run_on(
            machine,
            ledger,
            build,
            store,
            registry,
            pause,
            scratch_dir,
            stamp,
            *attempt,
            run_id,
            revision,
            *ttl,
            *with_publisher,
            stage,
            only,
        )
    }

    /// Seals a set of the two packages of the example for the stage `build`: the set document
    /// in the store, and the `stage` record that names it. `requires` is the `requires` label of
    /// the first package.
    pub fn seal(&mut self, requires: Option<&str>) -> Hash256 {
        self.seal_members(requires, &["ucdk-cli", "ucdk-lib"])
    }

    pub fn seal_members(&mut self, requires: Option<&str>, names: &[&str]) -> Hash256 {
        let members = names
            .iter()
            .enumerate()
            .map(|(at, name)| {
                let labels = match (at, requires) {
                    (0, Some(r)) => BTreeMap::from([("requires".to_owned(), r.to_owned())]),
                    _ => BTreeMap::new(),
                };
                Member {
                    manifest: Some(manifest(name, &labels)),
                }
            })
            .collect();
        let set = set_document::seal(ReleaseSet {
            schema_version: 1,
            revision: REVISION.to_owned(),
            platform: PLATFORM.to_owned(),
            members,
            set_hash: String::new(),
            producer: Some(Producer {
                stage: "build".to_owned(),
                environment: "build".to_owned(),
            }),
            produced_by: None,
        });
        self.put_set(&set, GENERATION);
        let hash = Hash256::from_hex(&set.set_hash).expect("a hash");
        // Another stage's record, met first by a search for the stage `build`.
        self.ledger.seed(
            &codec::Record::Stage(codec::Stage {
                stage: sorting_before("build", |n| format!("decoy-{n}")),
                attempt: 1,
                platform: None,
                set: None,
                generation: None,
                confirmed: false,
            }),
            State::Intent,
        );
        self.ledger.seed(
            &codec::Record::Stage(codec::Stage {
                stage: "build".to_owned(),
                attempt: 1,
                platform: Some(PLATFORM.to_owned()),
                set: Some(hash.clone()),
                generation: Some(GENERATION.to_owned()),
                confirmed: false,
            }),
            State::Intent,
        );
        hash
    }

    /// Another validation's record, met first by a search for the validation `name` of `stage`.
    pub fn decoy_validation(&mut self, stage: &str, name: &str) {
        let decoy = sorting_before(&format!("{stage}/{name}"), |n| format!("{stage}/decoy-{n}"));
        self.ledger.seed(
            &codec::Record::Validation(codec::Validation {
                stage: stage.to_owned(),
                validation: decoy.rsplit('/').next().expect("a name").to_owned(),
                set: None,
            }),
            State::Intent,
        );
    }

    /// Writes the set document of `set` into the store at `generation` of the stage `build`.
    pub fn put_set(&mut self, set: &ReleaseSet, generation: &str) {
        let key = StoreKey::set(&SetScope {
            stage: "build".to_owned(),
            platform: PLATFORM.to_owned(),
            revision: REVISION.to_owned(),
            generation: generation.to_owned(),
        })
        .expect("a key");
        self.store.inner.discard(&key).expect("discard");
        self.store
            .inner
            .begin(&key)
            .expect("begin")
            .put_file(SET_FILE, &set_document::encode(set))
            .expect("put");
    }

    /// Commits the packages `names` of the example to the registry, as a publish would.
    pub fn publish(&mut self, names: &[&str]) {
        let held = Held::default();
        for name in names {
            let blob = bytes_of(name);
            let coordinate = coordinate(name);
            let file = Blob { name, bytes: &blob };
            let item = StageItem {
                coordinate: &coordinate,
                file: &file,
                held: &held,
            };
            self.registry
                .inner
                .commit(&Grant::none(), &[item])
                .expect("commit");
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_on(
    machine: &Checked,
    ledger: &Chaos,
    build: &mut dyn BuildSystemSeam,
    store: &mut TestStore,
    registry: &mut Probe,
    pause: &mut FakePause,
    scratch: &Path,
    stamp: &BuildStamp,
    attempt: u32,
    run_id: &str,
    revision: &str,
    ttl: Duration,
    with_publisher: bool,
    stage: &str,
    only: &[&str],
) -> Outcome {
    let run = Run {
        id: run_id,
        attempt,
        revision,
        stamp,
    };
    let seams = Seams {
        build_system: build,
        store,
        channels: if with_publisher {
            BTreeMap::from([("registry".to_owned(), registry as &mut dyn Publisher)])
        } else {
            BTreeMap::new()
        },
        pause,
    };
    let request = Request {
        stage: stage.to_owned(),
        only: Selection::parse(&only.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
            .expect("selectors"),
    };
    Engine::new(machine, ledger, run, seams, scratch)
        .with_lease_ttl(ttl)
        .handle(&request)
}

// ---- members ----------------------------------------------------------------------------------------

/// The bytes the example gives the package `name`.
pub fn bytes_of(name: &str) -> Vec<u8> {
    format!("the bytes of {name}").into_bytes()
}

pub fn digest_of(bytes: &[u8]) -> String {
    Hash256::from_sha256(&Sha256::digest(bytes).into()).to_string()
}

pub fn manifest(name: &str, labels: &BTreeMap<String, String>) -> Manifest {
    let bytes = bytes_of(name);
    Manifest {
        schema_version: 1,
        r#type: ArtifactType::File as i32,
        name: name.to_owned(),
        version: "1.2.3".to_owned(),
        platform: PLATFORM.to_owned(),
        file: name.to_owned(),
        sha256: digest_of(&bytes),
        size: bytes.len() as u64,
        stamp: Some(Stamped {
            build_number: 41,
            source_commit: "abc123".to_owned(),
        }),
        metadata: Some(Metadata::FileMeta(FileMeta {
            labels: labels.clone(),
        })),
    }
}

pub fn coordinate(name: &str) -> Coordinate {
    Coordinate {
        channel: "registry".to_owned(),
        r#type: ArtifactType::File as i32,
        name: name.to_owned(),
        version: "1.2.3".to_owned(),
        platform: PLATFORM.to_owned(),
    }
}

/// The name of the row a Test action derives for the target `label`.
pub fn test_row(label: &str) -> String {
    format!("{label}: {}", ucdk_release_engine::TEST_ASSERTION)
}

/// The name of the row a read-back derives for the package `name` of the example.
pub fn readback_row(name: &str) -> String {
    format!(
        "registry/{name} 1.2.3 {PLATFORM} (FILE): {}",
        ucdk_release_engine::READBACK_ASSERTION
    )
}

/// A name made by `make` whose record sorts before the record named `target`, so a search of the
/// ledger meets the decoy first.
pub fn sorting_before(target: &str, make: impl Fn(usize) -> String) -> String {
    let at = |name: &str| RecordKey::of_name(name).segment();
    (0..)
        .map(make)
        .find(|name| at(name) < at(target))
        .expect("some name sorts first")
}

/// The mutex the hooks of a test share a value through.
pub type Shared<T> = Arc<Mutex<T>>;
