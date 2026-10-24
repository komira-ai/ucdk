// SPDX-License-Identifier: Apache-2.0
//! The world a command runs in: the cloud, the ledger and the two output buffers.

use ucdk_ledger::MemoryLedger;
use ucdk_provider_fake::{Fake, FakeConfig};

/// The providers `--provider` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderName {
    /// The fake cloud, hosting every kind.
    Fake,
    /// The fake cloud without `job`.
    FakeLimited,
}

impl ProviderName {
    /// The name as written in `--provider`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fake => "fake",
            Self::FakeLimited => "fake-limited",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        [Self::Fake, Self::FakeLimited]
            .into_iter()
            .find(|p| p.as_str() == text)
    }
}

/// Everything a run reads and changes besides the files it is given.
///
/// There is no state file: the clouds and the ledger live here, in memory, so running the
/// library twice over one `World` is a second run against what the first left behind. A test
/// simulates a crash that way, and seeds a fault or a foreign object by building the fake it
/// wants before the run. `run` appends what the command prints to `out` and `err`; the
/// binary writes them to the process streams when the command is done.
#[derive(Debug)]
pub struct World {
    /// The cloud `--provider=fake` runs against.
    pub fake: Fake,
    /// The cloud `--provider=fake-limited` runs against.
    pub fake_limited: Fake,
    /// The ledger both clouds share.
    pub ledger: MemoryLedger,
    /// What the command printed to standard output.
    pub out: String,
    /// What the command printed to standard error.
    pub err: String,
}

impl World {
    /// Empty clouds with no faults, an empty ledger and nothing printed.
    pub fn new() -> Self {
        Self {
            fake: Fake::new(FakeConfig::standard()),
            fake_limited: Fake::new(FakeConfig::limited()),
            ledger: MemoryLedger::new(),
            out: String::new(),
            err: String::new(),
        }
    }

    pub(crate) fn cloud(&mut self, provider: ProviderName) -> &mut Fake {
        match provider {
            ProviderName::Fake => &mut self.fake,
            ProviderName::FakeLimited => &mut self.fake_limited,
        }
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}
