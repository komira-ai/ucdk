// SPDX-License-Identifier: Apache-2.0
//! The API of the core: the `ucdk.api.v1` messages, the one function that shows a ledger, and
//! the table of exit classes.
//!
//! The messages are generated from `proto/ucdk/api/v1/api.proto` by `build.rs` into the build
//! directory, in [`v1`]. A host (the command line, the C ABI) does two things with this crate:
//!
//! - asks for a ledger to be shown with [`ledger_show`], which answers with a [`v1::Outcome`]
//!   that carries the exit class of the answer;
//! - reads the exit code of a class from [`v1::ExitClass::code`], the only place the mapping
//!   from class to code is written.

mod exit;
mod ledger_show;
mod ledger_view;

pub use ledger_show::{LedgerSource, ledger_show};

/// The generated messages of `ucdk.api.v1`.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/ucdk.api.v1.rs"));
}
