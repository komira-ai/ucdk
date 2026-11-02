// SPDX-License-Identifier: Apache-2.0
//! Seams that can be made to say something else.

use super::*;

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

    fn declare(&mut self, names: &[String]) -> Result<(), DeclareError> {
        self.inner.declare(names)
    }

    fn abort(&mut self, g: &Grant, set: &[StageItem<'_>]) -> Result<(), SeamError> {
        self.inner.abort(g, set)
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

/// What a [`Watched`] build system does with the report path after a test invocation.
pub type AfterTest<'a> = Box<dyn FnMut(&str) + Send + 'a>;

/// A build system that does something just before each test invocation, and records the report
/// path it was given and whether the directory of it existed.
pub struct Watched<'a> {
    pub inner: &'a mut FakeBuildSystem,
    pub before_test: Box<dyn FnMut() + Send + 'a>,
    /// Done with the report path just after each test invocation returned, when set.
    pub after_test: Option<AfterTest<'a>>,
    pub reports: Vec<(String, bool)>,
}

impl<'a> Watched<'a> {
    pub fn new(inner: &'a mut FakeBuildSystem, before_test: impl FnMut() + Send + 'a) -> Self {
        Self {
            inner,
            before_test: Box::new(before_test),
            after_test: None,
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
        let outcome = self.inner.test(inv, heartbeat);
        if let Some(after) = self.after_test.as_mut() {
            after(report);
        }
        outcome
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
