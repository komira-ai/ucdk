// SPDX-License-Identifier: Apache-2.0
//! The API of the core: the `ucdk.api.v1` messages, the code that converts them to and from the
//! engine's values, and the one table from outcome to exit class.
//!
//! The messages are generated from `proto/ucdk/api/v1/api.proto` by `build.rs` into the build
//! directory, in [`v1`]; the resources themselves stay in `ucdk-proto`. A host (the command
//! line today; a library and the SDKs later) does three things with this crate:
//!
//! - turns a request message into the engine's [`ucdk_engine::Request`]
//!   ([`v1::PlanRequest::to_request`], [`v1::StatusRequest::to_request`], [`v1::LeaksRequest::to_request`],
//!   [`v1::ApplyRequest::approved`] and [`v1::DestroyRequest::approved`]), or learns why it cannot ([`RequestError`]);
//! - turns what the engine answered into a [`v1::Outcome`] (`From<&ucdk_engine::Outcome>`), which
//!   carries the exit class of the answer;
//! - reads the exit class from [`exit_class`], the only place the mapping from outcome to exit
//!   code is written.
//!
//! A [`v1::Plan`] only goes out. A plan is made by the planner from the world as it is, and its
//! hash is what the approver pins, so a plan sent in by a caller would be a way around the hash:
//! a caller sends the hash, and the engine plans again and compares.

mod exit;
mod request;
mod response;

pub use exit::{exit_class, failure_class};
pub use request::{Approved, RequestError};
pub use response::catalog_response;

/// The generated messages of `ucdk.api.v1`.
#[allow(clippy::all, clippy::pedantic, missing_docs)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/ucdk.api.v1.rs"));
}
