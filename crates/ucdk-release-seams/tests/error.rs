// SPDX-License-Identifier: Apache-2.0
//! The classified error: what each variant promises and how each reads.

use ucdk_release::codec::GrantFault;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Backoff, Grant, SeamError};

fn every_variant() -> Vec<SeamError> {
    vec![
        SeamError::Refused(Reason::Credential),
        SeamError::Permanent("tail".to_owned()),
        SeamError::Transient("5xx".to_owned()),
        SeamError::RateLimited(Backoff { ticks: 3 }),
        SeamError::InFlight("staging".to_owned()),
        SeamError::Unknown("no answer".to_owned()),
        SeamError::GrantRefused(GrantFault::Expired),
    ]
}

#[test]
fn only_refused_permanent_rate_limited_and_grant_refused_promise_no_effect() {
    let promised: Vec<bool> = every_variant()
        .iter()
        .map(SeamError::promises_no_effect)
        .collect();
    assert_eq!(promised, [true, true, false, true, false, false, true]);
}

#[test]
fn errors_read_as_sentences() {
    let texts: Vec<String> = every_variant().iter().map(ToString::to_string).collect();
    assert_eq!(
        texts,
        [
            "refused (Credential); nothing happened",
            "failed for good, nothing happened: tail",
            "failed, and it may have landed: 5xx",
            "rate limited; retry after 3 ticks",
            "something may be underway: staging",
            "the answer was lost: no answer",
            "the grant was refused (Expired); nothing happened",
        ]
    );
    let boxed: Box<dyn std::error::Error> = Box::new(SeamError::Permanent("x".to_owned()));
    assert!(boxed.to_string().contains('x'));
}

#[test]
fn a_grant_carries_its_secret_and_stated_expiry_and_none_carries_neither() {
    let grant = Grant::issued(b"secret".to_vec(), 42);
    assert_eq!(
        (grant.secret(), grant.expires_ms(), grant.is_none()),
        (&b"secret"[..], Some(42), false)
    );
    let none = Grant::none();
    assert_eq!(
        (none.secret().len(), none.expires_ms(), none.is_none()),
        (0, None, true)
    );
}
