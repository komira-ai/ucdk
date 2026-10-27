// SPDX-License-Identifier: Apache-2.0
//! The classified error: what each variant promises and how each reads.

use ucdk_release::codec::GrantFault;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{Backoff, Grant, ObserveFailure, ObserveWhen, PAUSE_TICK_MS, SeamError};

#[test]
fn a_failed_preflight_observe_that_promises_no_effect_is_refused_not_indeterminate() {
    // Before any write, `Refused(r)` is refused with its own reason `r`, `Permanent` is refused
    // with `CHANNEL` (the same call fails again), and a denied grant is `CREDENTIAL`. Every other
    // failure leaves the registry unread and is `INDETERMINATE`.
    let want = [
        ObserveFailure::Refused(Reason::Credential),
        ObserveFailure::Refused(Reason::Channel),
        ObserveFailure::Indeterminate,
        ObserveFailure::Indeterminate,
        ObserveFailure::Indeterminate,
        ObserveFailure::Indeterminate,
        ObserveFailure::Indeterminate,
    ];
    let got: Vec<ObserveFailure> = every_variant()
        .iter()
        .map(|e| e.observe_failure(ObserveWhen::Preflight))
        .collect();
    assert_eq!(got, want);
}

#[test]
fn a_preflight_refusal_keeps_the_reason_the_registry_gave() {
    for reason in [Reason::Channel, Reason::Credential, Reason::PublishBurned] {
        assert_eq!(
            SeamError::Refused(reason).observe_failure(ObserveWhen::Preflight),
            ObserveFailure::Refused(reason)
        );
    }
}

#[test]
fn a_denied_grant_at_preflight_is_a_credential_refusal_and_any_other_grant_fault_is_not() {
    let got: Vec<ObserveFailure> = [
        GrantFault::Denied,
        GrantFault::Expired,
        GrantFault::Used,
        GrantFault::Unreachable,
    ]
    .into_iter()
    .map(|f| SeamError::GrantRefused(f).observe_failure(ObserveWhen::Preflight))
    .collect();
    assert_eq!(
        got,
        [
            ObserveFailure::Refused(Reason::Credential),
            ObserveFailure::Indeterminate,
            ObserveFailure::Indeterminate,
            ObserveFailure::Indeterminate,
        ]
    );
}

#[test]
fn a_failed_observe_after_an_effect_may_have_landed_is_always_indeterminate() {
    // After the S3 `Duplicate`, a commit answer or a read-back poll, the outcome concerns the
    // effect, which is unknown: a refusal of the observe, a permanent failure and a denied grant
    // say nothing about it, so none of them is a refusal of the action.
    let mut every = every_variant();
    every.extend([
        SeamError::Refused(Reason::Channel),
        SeamError::GrantRefused(GrantFault::Denied),
        SeamError::GrantRefused(GrantFault::Used),
        SeamError::GrantRefused(GrantFault::Unreachable),
    ]);
    for error in every {
        assert_eq!(
            error.observe_failure(ObserveWhen::AfterEffect),
            ObserveFailure::Indeterminate,
            "{error}"
        );
    }
}

#[test]
fn a_pause_tick_is_one_second_and_a_backoff_converts_a_wait_through_it() {
    // The documented value of the tick, and the one conversion every registry uses: a wait told
    // in milliseconds or seconds becomes whole ticks of `PAUSE_TICK_MS`, rounded up so a
    // registry never asks for less than it was told, and saturating.
    assert_eq!(PAUSE_TICK_MS, 1000);
    let tick = u64::from(PAUSE_TICK_MS);
    for (ms, ticks) in [
        (0, 0),
        (1, 1),
        (tick - 1, 1),
        (tick, 1),
        (tick + 1, 2),
        (7 * tick, 7),
        (u64::MAX, u32::MAX),
    ] {
        assert_eq!(Backoff::after_millis(ms), Backoff { ticks }, "{ms} ms");
    }
    assert_eq!(Backoff::after_seconds(0), Backoff { ticks: 0 });
    assert_eq!(Backoff::after_seconds(7), Backoff { ticks: 7 });
    assert_eq!(
        Backoff::after_seconds(u32::MAX),
        Backoff { ticks: u32::MAX }
    );
}

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
