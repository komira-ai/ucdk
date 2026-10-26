// SPDX-License-Identifier: Apache-2.0
//! `ledger show`: one ledger document as data. The C ABI and the command line both call
//! [`ledger_show`], so the answer, its exit class and the text of its error have one producer.

use std::path::Path;
use std::sync::Arc;

use ucdk_core::Clock;
use ucdk_ledger::{FileLedger, Ledger};

use crate::ledger_view;
use crate::v1::{Error, ErrorCode, ExitClass, Outcome, outcome};

/// Where the ledger to show is.
pub enum LedgerSource<'a> {
    /// A ledger file, opened read-only for the call: no lock file is made, no lock is taken and
    /// no file is changed.
    File(&'a Path),
    /// A ledger the caller already holds. It is read as it is, by its own clock.
    Open(&'a dyn Ledger),
}

fn failed(message: String) -> Outcome {
    Outcome {
        result: Some(outcome::Result::Error(Error {
            code: ErrorCode::Ledger as i32,
            message,
        })),
        exit: ExitClass::FailedNoEffect as i32,
    }
}

/// Shows the document of `source`: the token, the highest fence, the lease and the records. A
/// lease is `Live`, `Lapsing` or `Lapsed` by the reading of the ledger's clock taken with the
/// read (for a file, `clock`, which the file is opened with), and `yours` when its holder is
/// `holder`.
///
/// The call takes no lease and changes nothing. A ledger that cannot be opened or read (a file
/// that is missing, corrupt or of a newer major, a ledger whose read fails) is an `error`
/// outcome with `ErrorCode::Ledger` and the class `FailedNoEffect`; a newer major is named in
/// the message.
pub fn ledger_show(source: &LedgerSource<'_>, holder: &str, clock: Arc<dyn Clock>) -> Outcome {
    let read = match source {
        LedgerSource::File(path) => FileLedger::open_read_only_with_clock(path, clock)
            .map_err(|e| e.to_string())
            .and_then(|ledger| ledger.read().map_err(|e| e.to_string())),
        LedgerSource::Open(ledger) => ledger.read().map_err(|e| e.to_string()),
    };
    match read {
        Ok(snapshot) => Outcome {
            result: Some(outcome::Result::LedgerShow(ledger_view::response(
                &snapshot, holder,
            ))),
            exit: ExitClass::Done as i32,
        },
        Err(message) => failed(message),
    }
}
