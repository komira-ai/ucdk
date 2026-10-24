// SPDX-License-Identifier: Apache-2.0
//! The fake as a subject of the contract kit.

use ucdk_provider::kit::Harness;
use ucdk_provider::{Kind, PhysicalId, Stamp};

use crate::config::FakeConfig;
use crate::fake::Fake;

/// Builds fakes for the kit. `FakeHarness::standard()` hosts `service` and `job`;
/// `FakeHarness::limited()` has no `job` and declares it unsupported.
#[derive(Debug, Clone)]
pub struct FakeHarness {
    config: FakeConfig,
}

impl FakeHarness {
    pub fn standard() -> Self {
        Self {
            config: FakeConfig::standard(),
        }
    }

    pub fn limited() -> Self {
        Self {
            config: FakeConfig::limited(),
        }
    }
}

impl Harness for FakeHarness {
    type Provider = Fake;

    fn fresh(&self) -> Fake {
        Fake::new(self.config.clone())
    }

    fn kinds(&self) -> Vec<Kind> {
        [Kind::service(), Kind::job()]
            .into_iter()
            .filter(|k| !self.config.unsupported.contains(k))
            .collect()
    }

    fn unsupported(&self) -> Vec<Kind> {
        self.config.unsupported.iter().cloned().collect()
    }

    fn add_foreign(&self, provider: &mut Fake, kind: &Kind, name: &str) {
        provider.add_foreign(kind, name, None);
    }

    fn rename(&self, provider: &mut Fake, physical: &PhysicalId, new_name: &str) {
        provider.rename(physical, new_name);
    }

    fn stamp_at_birth(&self, provider: &Fake, physical: &PhysicalId) -> Option<Stamp> {
        provider.birth_stamp(physical)
    }
}
