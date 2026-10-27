// SPDX-License-Identifier: Apache-2.0
//! The scenario of the grant source kit about the secret of a grant: it is never printed, cloned
//! or logged.

use std::fmt::{Debug, Display};
use std::marker::PhantomData;

use ucdk_release::codec::GrantPhase;

use super::grant_source::{
    GrantSourceRig, LOCATION, PHASES, declaration, exceptional_job, federated_repository,
    matching_job, request, setup,
};
use super::{coordinate, must, must_err, same};
use crate::{Grant, GrantRequest, GrantSource};

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
    let elsewhere = declaration(LOCATION, |declared| {
        "elsewhere".clone_into(&mut declared.subject);
    });
    // Eight coordinates, so a quarter of the secret is longer than anything the issuer may
    // legitimately record about one of them (the scope, the location, a coordinate's own text).
    let coords = ["a", "b", "c", "d", "e", "f", "g", "h"].map(coordinate);
    // An ordinary run and an exceptional run each get a grant of every phase from one source,
    // after a refusal, so a secret kept out of the log only for the first grant, for a stage
    // grant, for an ordinary run or for a source that has not refused is found.
    for as_exceptional in [false, true] {
        let job = if as_exceptional {
            exceptional_job()
        } else {
            matching_job()
        };
        let mut source = rig.source(&setup(job));
        must_err(
            source.grant(&request(&elsewhere, GrantPhase::Stage, &coords)),
            "a grant for a repository whose claims the job does not meet",
        );
        let mut grants = Vec::new();
        for phase in PHASES {
            let req = GrantRequest {
                exceptional: as_exceptional,
                ..request(&repository, phase, &coords)
            };
            let answer = source.grant(&req);
            // That an exceptional run is granted is the exceptional scenario's to hold.
            if as_exceptional {
                grants.extend(answer.ok().map(|grant| (phase, grant)));
            } else {
                grants.push((phase, must(answer, "grant")));
            }
        }
        let log = rig.log(&source);
        for (phase, grant) in &grants {
            let secret = grant.secret();
            same(
                secret.is_empty(),
                false,
                "a grant with no secret proves nothing",
            );
            // The secret in any form is not in the log: a stretch of at least a quarter of it,
            // as it is, reversed, in hex of either case, in base64 or with a space after each
            // byte.
            let reversed: Vec<u8> = secret.iter().rev().copied().collect();
            let hex: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
            let upper = hex.to_uppercase();
            let spaced: Vec<u8> = secret.iter().flat_map(|byte| [*byte, b' ']).collect();
            let encoded = base64(secret);
            for (form, bytes) in [
                ("as it is", secret),
                ("reversed", reversed.as_slice()),
                ("in lower-case hex", hex.as_bytes()),
                ("in upper-case hex", upper.as_bytes()),
                ("in base64", encoded.as_slice()),
                ("with a space after each byte", spaced.as_slice()),
            ] {
                same(
                    holds_a_stretch(&log, bytes, bytes.len().div_ceil(4)),
                    false,
                    &format!(
                        "a quarter of the secret of a {phase:?} grant {form} in what the issuer recorded (exceptional {as_exceptional})"
                    ),
                );
            }
        }
    }
}

/// `bytes` in base64 (the standard alphabet, without padding, which a stretch of the text does
/// not need).
fn base64(bytes: &[u8]) -> Vec<u8> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut word, mut bits) = (0u32, 0u32);
    let mut out = Vec::new();
    for byte in bytes {
        word = (word << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(ALPHABET[((word >> bits) & 63) as usize]);
        }
    }
    out.push(ALPHABET[((word << (6 - bits)) & 63) as usize]);
    out
}
