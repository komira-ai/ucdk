// SPDX-License-Identifier: Apache-2.0
//! Which kinds a fake hosts.

use ucdk_provider::Kind;
use ucdk_provider_fake::{Fake, FakeConfig};

#[test]
fn hosts_reports_every_kind_but_the_unsupported_ones() {
    let standard = Fake::new(FakeConfig::standard());
    assert!(standard.hosts(&Kind::service()) && standard.hosts(&Kind::job()));

    let limited = Fake::new(FakeConfig::limited());
    assert!(limited.hosts(&Kind::service()));
    assert!(!limited.hosts(&Kind::job()));
}
