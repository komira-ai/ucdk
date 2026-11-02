// SPDX-License-Identifier: Apache-2.0
//! A ledger that can be made to fail.

use super::*;

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
