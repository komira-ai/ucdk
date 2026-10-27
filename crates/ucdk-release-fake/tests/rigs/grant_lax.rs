// SPDX-License-Identifier: Apache-2.0
//! What a stub of the grant-source rig does to the coordinates of a request, to the limit, to
//! the expiry of a grant and to the refusal an issuer gives: the lax readings of the stubs.

use std::collections::BTreeSet;

use ucdk_release::codec::{GrantFault, GrantPhase};
use ucdk_release_proto::v1::Coordinate;
use ucdk_release_seams::kit::coordinate;

use super::grant_source::Flaw;

/// The coordinates a stub covers in place of those asked; none when the stub covers them as
/// asked.
pub fn covered_in_place_of(
    flaw: Option<Flaw>,
    exceptional: bool,
    asked: &[Coordinate],
) -> Option<Vec<Coordinate>> {
    let each = |change: fn(&str) -> String| -> Vec<Coordinate> {
        asked
            .iter()
            .map(|c| Coordinate {
                channel: change(&c.channel),
                name: change(&c.name),
                version: change(&c.version),
                platform: change(&c.platform),
                ..c.clone()
            })
            .collect()
    };
    // A source that joins the names of a list with a separator and a registry that splits the
    // list at it read a name that holds the separator as several coordinates.
    let split_names = |at: char| -> Vec<Coordinate> {
        asked
            .iter()
            .flat_map(|c| {
                c.name.split(at).map(|name| Coordinate {
                    name: name.to_owned(),
                    ..c.clone()
                })
            })
            .collect()
    };
    match flaw? {
        Flaw::NormalizesCoordinateFields => {
            Some(asked.iter().map(|c| coordinate(&c.name)).collect())
        }
        Flaw::CoordinatesLowerCased => Some(each(str::to_lowercase)),
        Flaw::CoordinatesTrimmed => Some(each(|text| text.trim().to_owned())),
        Flaw::CoordinatesCutAtASlash => Some(each(|text| {
            text.split('/').next().unwrap_or_default().to_owned()
        })),
        Flaw::CoordinatesCutAtANewline => Some(each(|text| {
            text.split('\n').next().unwrap_or_default().to_owned()
        })),
        Flaw::NamesSplitAtAComma => Some(split_names(',')),
        Flaw::NamesSplitAtANewline => Some(split_names('\n')),
        Flaw::ExceptionalCoversWiderCoordinates if exceptional => {
            Some(asked.iter().cloned().chain([coordinate("*")]).collect())
        }
        _ => None,
    }
}

/// Whether a stub finds `asked` over the `limit` where the issuer it wraps, which has no limit of
/// its own for that stub, would not.
pub fn lax_limit_is_exceeded(
    flaw: Option<Flaw>,
    exceptional: bool,
    limit: usize,
    asked: &[Coordinate],
) -> bool {
    match flaw {
        Some(Flaw::LimitCountsDistinctNames) => {
            asked
                .iter()
                .map(|c| c.name.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                > limit
        }
        Some(Flaw::LimitCountsDistinctCoordinates) => {
            let mut distinct: Vec<&Coordinate> = Vec::new();
            for c in asked {
                if !distinct.contains(&c) {
                    distinct.push(c);
                }
            }
            distinct.len() > limit
        }
        Some(Flaw::ExceptionalIgnoresTheLimit) => !exceptional && asked.len() > limit,
        _ => false,
    }
}

/// The expiry a stub states for a grant the issuer stated to expire at `stated`, when the clock
/// reads `now`.
pub fn lax_expiry(flaw: Option<Flaw>, exceptional: bool, stated: u64, now: u64) -> u64 {
    match flaw {
        Some(Flaw::ExpiryFlooredAtASecondAhead) => stated.max(now.saturating_add(1_000)),
        Some(Flaw::ExpiredGrantReportedLive) if stated <= now => now.saturating_add(1_000),
        Some(Flaw::LifetimeCappedAtAnHour) => stated.min(now.saturating_add(3_600_000)),
        Some(Flaw::ExceptionalGrantNeverExpires) if exceptional => u64::MAX,
        _ => stated,
    }
}

/// Whether a stub reports the refusal `fault` of a request of `phase` as `Denied`.
pub fn reported_as_denied(flaw: Flaw, fault: GrantFault, phase: GrantPhase) -> bool {
    matches!(
        (flaw, fault, phase),
        (
            Flaw::UsedIsDeniedForCommit,
            GrantFault::Used,
            GrantPhase::Commit
        ) | (
            Flaw::UsedIsDeniedForRead,
            GrantFault::Used,
            GrantPhase::Read
        ) | (Flaw::ExpiredIsDenied, GrantFault::Expired, _)
            | (
                Flaw::UnreachableIsDeniedOutsideStage,
                GrantFault::Unreachable,
                GrantPhase::Commit | GrantPhase::Read,
            )
    )
}

/// Whether a stub writes the stage scope into a grant of `phase` that was issued for its own.
pub fn has_the_stage_scope(flaw: Flaw, phase: GrantPhase, exceptional: bool) -> bool {
    match flaw {
        Flaw::CommitGrantHasTheStageScope => phase == GrantPhase::Commit,
        Flaw::ExceptionalCommitHasTheStageScope => phase == GrantPhase::Commit && exceptional,
        Flaw::ReadIsIssuedAsStage => phase == GrantPhase::Read && !exceptional,
        _ => false,
    }
}

/// A text as a lax issuer folds it before it compares: full-width letters, the Kelvin sign and
/// the long s become the letters they resemble.
pub fn folded(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\u{212a}' => 'k',
            '\u{17f}' => 's',
            c => char::from_u32(u32::from(c).wrapping_sub(0xFEE0))
                .filter(char::is_ascii_graphic)
                .unwrap_or(c),
        })
        .collect()
}

/// A text without its zero-width characters.
pub fn without_invisible(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}' | '\u{2060}'
            )
        })
        .collect()
}

/// A text up to its first NUL.
pub fn up_to_nul(text: &str) -> &str {
    text.split('\0').next().unwrap_or_default()
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    bytes
        .chunks(3)
        .flat_map(|chunk| {
            let word = chunk.iter().enumerate().fold(0u32, |word, (at, byte)| {
                word | (u32::from(*byte) << (16 - 8 * at))
            });
            (0..4).map(move |at| {
                if at <= chunk.len() {
                    ALPHABET[((word >> (18 - 6 * at)) & 63) as usize] as char
                } else {
                    '='
                }
            })
        })
        .collect()
}
