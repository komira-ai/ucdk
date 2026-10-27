// SPDX-License-Identifier: Apache-2.0
//! The checks of the grant source kit about how the claims of a job are compared with the
//! declaration: every pinned claim, and the claims that only resemble the declared ones.

use std::collections::BTreeMap;

use ucdk_release_proto::v1::Repository;

use super::grant_source::{
    GrantSourceRig, JobClaims, LOCATION, PHASES, declaration, denied, exceptional_job,
    federated_repository, matching_job, request, setup,
};
use super::{coordinate, must, must_err, same};
use crate::{GrantRequest, GrantSource};

/// Every pinned claim of the declaration is compared, the last as the first, and a job that
/// presents a claim the declaration does not pin is not the job it declares.
pub(super) fn several_pinned_claims_are_each_compared_and_none_may_be_added<R: GrantSourceRig>(
    rig: &R,
) {
    let coords = [coordinate("a")];
    let pinned = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    };
    let pins = |pairs: &[(&str, &str)]| JobClaims {
        pinned: pinned(pairs),
        ..matching_job()
    };
    let two = declaration(LOCATION, |declared| {
        declared.pinned_claims = pinned(&[("ref", "main"), ("workflow", "release")]);
    });
    let one = federated_repository();
    for phase in PHASES {
        let mut source = rig.source(&setup(pins(&[("ref", "main"), ("workflow", "release")])));
        must(
            source.grant(&request(&two, phase, &coords)),
            &format!("a {phase:?} grant for a job that presents both pinned claims"),
        );
        let differing = [
            (
                &two,
                "the second of two pinned claims",
                pins(&[("ref", "main"), ("workflow", "other")]),
            ),
            (
                &two,
                "the first of two pinned claims",
                pins(&[("ref", "topic"), ("workflow", "release")]),
            ),
            (
                &two,
                "one of two pinned claims missing",
                pins(&[("ref", "main")]),
            ),
            (
                &two,
                "a third claim the declaration does not pin",
                pins(&[("ref", "main"), ("workflow", "release"), ("extra", "x")]),
            ),
            (
                &one,
                "a second claim the declaration does not pin",
                pins(&[("ref", "main"), ("extra", "x")]),
            ),
        ];
        for (repository, which, job) in differing {
            let mut source = rig.source(&setup(job));
            let refusal = must_err(
                source.grant(&request(repository, phase, &coords)),
                &format!("a {phase:?} grant for a job with {which}"),
            );
            same(
                denied(&refusal),
                true,
                &format!("a job with {which} is Denied for a {phase:?} grant"),
            );
            same(
                rig.exchanges(&source),
                0,
                &format!("the exchanges a job with {which} spent on a {phase:?} grant"),
            );
        }
    }
}

/// What a source answered for one request does not decide what it answers for the next: a
/// request for a repository whose claims the job does not meet is refused as often as it is
/// asked, whatever was granted before it and whichever repository was asked about first, and a
/// refusal does not stop the grants that follow it.
pub(super) fn an_earlier_request_does_not_decide_the_next<R: GrantSourceRig>(rig: &R) {
    let coords = [coordinate("a")];
    let admitted = federated_repository();
    let other = declaration("registry/other", |declared| {
        "elsewhere".clone_into(&mut declared.subject);
    });
    for phase in PHASES {
        let mut source = rig.source(&setup(matching_job()));
        for (step, (repository, granted)) in [
            (&admitted, true),
            (&other, false),
            (&other, false),
            (&admitted, true),
            (&other, false),
        ]
        .into_iter()
        .enumerate()
        {
            let answer = source.grant(&request(repository, phase, &coords));
            let what =
                format!("request {step} of one source, a {phase:?} grant, granted {granted}");
            if granted {
                must(answer, &what);
            } else {
                same(denied(&must_err(answer, &what)), true, &what);
            }
        }
        same(
            rig.exchanges(&source),
            2,
            &format!("the exchanges of the two {phase:?} grants of one source"),
        );
    }
}

/// A claim a job presents that the declaration compares.
#[derive(Debug, Clone, Copy)]
enum Claim {
    Subject,
    Audience,
    Environment,
    PinnedName,
    PinnedValue,
}

const CLAIMS: [Claim; 5] = [
    Claim::Subject,
    Claim::Audience,
    Claim::Environment,
    Claim::PinnedName,
    Claim::PinnedValue,
];

impl Claim {
    /// What `job` presents for this claim (its one pinned claim, for the pinned ones).
    fn of(self, job: &JobClaims) -> &str {
        let pinned = job.pinned.iter().next();
        match self {
            Self::Subject => &job.subject,
            Self::Audience => &job.audience,
            Self::Environment => &job.environment,
            Self::PinnedName => pinned.map(|(key, _)| key.as_str()).unwrap_or_default(),
            Self::PinnedValue => pinned.map(|(_, value)| value.as_str()).unwrap_or_default(),
        }
    }

    /// `job` with `value` in place of what it presents for this claim.
    fn set(self, job: &JobClaims, value: String) -> JobClaims {
        let (key, kept) = (Self::PinnedName.of(job), Self::PinnedValue.of(job));
        let mut changed = job.clone();
        match self {
            Self::Subject => changed.subject = value,
            Self::Audience => changed.audience = value,
            Self::Environment => changed.environment = value,
            Self::PinnedName => changed.pinned = BTreeMap::from([(value, kept.to_owned())]),
            Self::PinnedValue => changed.pinned = BTreeMap::from([(key.to_owned(), value)]),
        }
        changed
    }
}

/// Texts that are not `declared` and that a lax comparison takes for it: `declared` with
/// something added before it, after it or around it, with white space around it, with a
/// zero-width character or a NUL in it or beside it, in full-width letters, and with the long s
/// and the Kelvin sign for the letters they look like. A text the change leaves as it was (no
/// `s`, no `k`) is left out.
fn lookalikes(declared: &str) -> Vec<(&'static str, String)> {
    let (head, tail) = declared.split_at(declared.len().min(1));
    let full_width: String = declared
        .chars()
        .map(|c| char::from_u32(u32::from(c) + 0xFEE0).unwrap_or(c))
        .collect();
    let look_alike_letters: String = declared
        .chars()
        .map(|c| match c {
            's' => '\u{17f}',
            'k' => '\u{212a}',
            other => other,
        })
        .collect();
    Vec::from([
        ("with a suffix", format!("{declared}-evil")),
        ("with a prefix", format!("evil-{declared}")),
        ("inside other text", format!("evil-{declared}-evil")),
        ("with a space before it", format!(" {declared}")),
        ("with a space after it", format!("{declared} ")),
        ("with a newline after it", format!("{declared}\n")),
        (
            "with a zero-width space inside it",
            format!("{head}\u{200b}{tail}"),
        ),
        (
            "with a zero-width no-break space before it",
            format!("\u{feff}{declared}"),
        ),
        ("with a word joiner after it", format!("{declared}\u{2060}")),
        ("with a NUL inside it", format!("{head}\0{tail}")),
        ("with a NUL and more after it", format!("{declared}\0evil")),
        ("in full-width letters", full_width),
        ("with a long s and a Kelvin sign", look_alike_letters),
    ])
    .into_iter()
    .filter(|(_, text)| text != declared)
    .collect()
}

/// A declaration whose every claim holds an `s` and a `k`, so each look-alike letter has a place
/// to stand, with the claims of the ordinary job and of the exceptional job it admits.
fn keen_declaration() -> (Repository, JobClaims, JobClaims) {
    let repository = declaration(LOCATION, |declared| {
        "skis".clone_into(&mut declared.subject);
        "kiosk".clone_into(&mut declared.audience);
        "kayak".clone_into(&mut declared.environment);
        "ski-exceptional".clone_into(&mut declared.exceptional_subject);
        "kayak-exceptional".clone_into(&mut declared.exceptional_environment);
        declared.pinned_claims = BTreeMap::from([("task".to_owned(), "ski".to_owned())]);
    });
    let ordinary = JobClaims {
        subject: "skis".to_owned(),
        audience: "kiosk".to_owned(),
        environment: "kayak".to_owned(),
        pinned: BTreeMap::from([("task".to_owned(), "ski".to_owned())]),
    };
    let exceptional = JobClaims {
        subject: "ski-exceptional".to_owned(),
        environment: "kayak-exceptional".to_owned(),
        ..ordinary.clone()
    };
    (repository, ordinary, exceptional)
}

/// A claim is compared as it is written: a job whose claim only contains the declared one, starts
/// or ends with it, has white space around it or spells it with look-alike characters is not the
/// job the declaration names, for each claim, each phase and an exceptional run.
pub(super) fn claims_that_only_resemble_the_declared_are_refused<R: GrantSourceRig>(rig: &R) {
    let coords = [coordinate("a")];
    for (repository, ordinary, exceptional) in [
        (federated_repository(), matching_job(), exceptional_job()),
        keen_declaration(),
    ] {
        for (as_exceptional, job) in [(false, ordinary), (true, exceptional)] {
            for phase in PHASES {
                let ask = |claims: &JobClaims| {
                    let mut source = rig.source(&setup(claims.clone()));
                    let req = GrantRequest {
                        exceptional: as_exceptional,
                        ..request(&repository, phase, &coords)
                    };
                    let answer = source.grant(&req);
                    (answer, rig.exchanges(&source))
                };
                // That the exceptional job is admitted is the exceptional scenario's to hold.
                if !as_exceptional {
                    must(
                        ask(&job).0,
                        &format!("a {phase:?} grant for the job the declaration names"),
                    );
                }
                for claim in CLAIMS {
                    for (what, text) in lookalikes(claim.of(&job)) {
                        let (answer, exchanges) = ask(&claim.set(&job, text));
                        let refusal = must_err(
                            answer,
                            &format!("a {phase:?} grant for a job whose {claim:?} is {what}"),
                        );
                        same(
                            denied(&refusal),
                            true,
                            &format!("a {claim:?} {what} is Denied for a {phase:?} grant"),
                        );
                        same(
                            exchanges,
                            0,
                            &format!("the exchanges a {claim:?} {what} spent on a {phase:?} grant"),
                        );
                    }
                }
            }
        }
    }
}
