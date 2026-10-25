// SPDX-License-Identifier: Apache-2.0
//! Which kinds a fake hosts.

use ucdk_provider::{Kind, Provider};
use ucdk_provider_fake::{Fake, FakeConfig};

#[test]
fn hosts_reports_every_kind_but_the_unsupported_ones() {
    let standard = Fake::new(FakeConfig::standard());
    assert!(standard.hosts(&Kind::service()) && standard.hosts(&Kind::job()));

    let limited = Fake::new(FakeConfig::limited());
    assert!(limited.hosts(&Kind::service()));
    assert!(!limited.hosts(&Kind::job()));
}

/// Proves: the trait method answers as the fake's own `hosts` does, since the engine asks the
/// trait. Catches a trait impl that says yes to everything.
#[test]
fn the_trait_method_agrees_with_the_inherent_one() {
    let limited = Fake::new(FakeConfig::limited());
    assert!(Provider::hosts(&limited, &Kind::service()));
    assert!(!Provider::hosts(&limited, &Kind::job()));
}
