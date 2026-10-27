// SPDX-License-Identifier: Apache-2.0
//! The scenario of the grant source kit about the secret of a grant: it is never printed, cloned
//! or logged.

use std::fmt::{Debug, Display};
use std::marker::PhantomData;

use ucdk_release::codec::GrantPhase;

use super::grant_source::{GrantSourceRig, federated_repository, matching_job, request, setup};
use super::{coordinate, must, same};
use crate::{Grant, GrantSource};

/// Whether a type has a trait, found without naming it as a bound: the inherent constant is
/// there only when the bound holds, and the trait's default stands in when it does not.
struct Probe<T>(PhantomData<T>);

trait Lacks {
    const HAS: bool = false;
}

impl<T> Lacks for Probe<T> {}

// The constants below are read only for a type that has the trait; `Grant` has none of them,
// which is what the scenario holds, so they are never read here.
#[allow(dead_code)]
impl<T: Debug> Probe<T> {
    const HAS: bool = true;
}

struct DisplayProbe<T>(PhantomData<T>);

impl<T> Lacks for DisplayProbe<T> {}

#[allow(dead_code)]
impl<T: Display> DisplayProbe<T> {
    const HAS: bool = true;
}

struct CloneProbe<T>(PhantomData<T>);

impl<T> Lacks for CloneProbe<T> {}

#[allow(dead_code)]
impl<T: Clone> CloneProbe<T> {
    const HAS: bool = true;
}

/// Whether `log` holds a stretch of `len` bytes of `bytes`.
fn holds_a_stretch(log: &[u8], bytes: &[u8], len: usize) -> bool {
    bytes
        .windows(len)
        .any(|stretch| log.windows(len).any(|seen| seen == stretch))
}

pub(super) fn a_grant_is_never_printed_cloned_or_logged<R: GrantSourceRig>(rig: &R) {
    same(<Probe<Grant>>::HAS, false, "Grant implements Debug");
    same(
        <DisplayProbe<Grant>>::HAS,
        false,
        "Grant implements Display",
    );
    same(<CloneProbe<Grant>>::HAS, false, "Grant implements Clone");
    let repository = federated_repository();
    // Several coordinates, so the secret is longer than anything the issuer may legitimately
    // record about one coordinate (the scope, the location, a coordinate's own text).
    let coords = [coordinate("a"), coordinate("b"), coordinate("c")];
    let mut source = rig.source(&setup(matching_job()));
    let grant = must(
        source.grant(&request(&repository, GrantPhase::Stage, &coords)),
        "grant",
    );
    let secret = grant.secret();
    same(
        secret.is_empty(),
        false,
        "a grant with no secret proves nothing",
    );
    let log = rig.log(&source);
    // The secret in any form is not in the log: whole or in a stretch of at least half of it,
    // as it is or reversed, and in hex in either case.
    let reversed: Vec<u8> = secret.iter().rev().copied().collect();
    let hex: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
    let upper = hex.to_uppercase();
    for (form, bytes) in [
        ("as it is", secret),
        ("reversed", reversed.as_slice()),
        ("in lower-case hex", hex.as_bytes()),
        ("in upper-case hex", upper.as_bytes()),
    ] {
        same(
            holds_a_stretch(&log, bytes, bytes.len().div_ceil(2)),
            false,
            &format!("half the secret of a grant {form} in what the issuer recorded"),
        );
    }
}
