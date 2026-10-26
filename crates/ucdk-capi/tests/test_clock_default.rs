// SPDX-License-Identifier: Apache-2.0
#![cfg(not(feature = "test-clock"))]
//! In a default build (the one a release ships), a frozen clock cannot be asked for: a caller
//! that could pin the time the ledger judges leases by would defeat the lease and the fence.
//! `cargo xtask check` runs this file; it is empty when the `test-clock` feature is on.

mod common;

use common::{error_of, fake_config, open};
use ucdk_capi::{INVALID_ARGUMENT, OK};

/// Proves: a non-zero `test_clock_ms` is refused with INVALID_ARGUMENT, names the feature, and
/// opens no session, while zero (the wall clock) opens. Catches a production library that
/// honours a clock chosen by its caller.
#[test]
fn a_default_build_refuses_a_fixed_clock() {
    let mut config = fake_config();
    config.test_clock_ms = 1;
    let refused = open(&config);
    assert_eq!(refused.status, INVALID_ARGUMENT);
    assert!(refused.session.is_null());
    assert!(
        error_of(&refused.error).message.contains("test-clock"),
        "the refusal says which feature it lacks"
    );

    config.test_clock_ms = 0;
    let opened = open(&config);
    assert_eq!(opened.status, OK);
    ucdk_capi::ucdk_session_close(opened.session);
}
