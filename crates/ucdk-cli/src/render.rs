// SPDX-License-Identifier: Apache-2.0
//! The text `ucdk ledger show` prints: one fact per line, every value escaped exactly as the
//! ledger file escapes it.

use std::fmt::Write as _;

use ucdk_api::v1::{LeaseState, LedgerShowResponse, RecordOrigin, RecordState};
use ucdk_ledger::escape_token;

fn lease_state(state: i32) -> &'static str {
    match LeaseState::try_from(state) {
        Ok(LeaseState::Live) => "live",
        Ok(LeaseState::Lapsing) => "lapsing",
        Ok(LeaseState::Lapsed) => "lapsed",
        _ => "unknown",
    }
}

fn origin(origin: i32) -> &'static str {
    match RecordOrigin::try_from(origin) {
        Ok(RecordOrigin::Created) => "created",
        Ok(RecordOrigin::Adopted) => "adopted",
        _ => "unknown",
    }
}

fn state(state: i32) -> &'static str {
    match RecordState::try_from(state) {
        Ok(RecordState::Intent) => "intent",
        Ok(RecordState::Confirmed) => "confirmed",
        Ok(RecordState::Reaping) => "reaping",
        _ => "unknown",
    }
}

/// The document as text, every line beginning with `token`, `fence`, `lease` or `record`:
///
/// ```text
/// token <text>
/// fence <n>
/// lease none
/// lease <holder> expires_ms <n> <live|lapsing|lapsed> <yours|other>
/// record <path> <kind> <name> <created|adopted> <state> physical=<text> deps=<a,b> <key>=<value> ...
/// ```
///
/// Every value is one token: the bytes other than letters, digits and `._-/:` print as `%XX` and
/// the empty text as a lone `%` (`ucdk_ledger::escape_token`, the function that writes the
/// file), so a value that holds a newline, a space, a comma or `=` can neither start another
/// line, nor forge a `lease` or `record` line, nor shift a field. A state or an origin the
/// library does not know prints as `unknown`.
pub fn ledger_text(response: &LedgerShowResponse) -> String {
    let mut text = format!(
        "token {}\nfence {}\n",
        escape_token(&response.token),
        response.fence
    );
    match &response.lease {
        None => text.push_str("lease none\n"),
        Some(lease) => {
            let _ = writeln!(
                text,
                "lease {} expires_ms {} {} {}",
                escape_token(&lease.holder),
                lease.expires_ms,
                lease_state(lease.state),
                if lease.yours { "yours" } else { "other" }
            );
        }
    }
    for record in &response.records {
        let deps: Vec<String> = record.deps.iter().map(|dep| escape_token(dep)).collect();
        let _ = write!(
            text,
            "record {} {} {} {} {} physical={} deps={}",
            escape_token(&record.path),
            escape_token(&record.kind),
            escape_token(&record.name),
            origin(record.origin),
            state(record.state),
            escape_token(&record.physical),
            deps.join(",")
        );
        for pair in &record.outputs {
            let _ = write!(
                text,
                " {}={}",
                escape_token(&pair.key),
                escape_token(&pair.value)
            );
        }
        text.push('\n');
    }
    text
}
