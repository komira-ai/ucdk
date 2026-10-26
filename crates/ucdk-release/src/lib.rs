// SPDX-License-Identifier: Apache-2.0
//! The release model: the rules and decisions of a release that need no clock, no store and
//! no registry, so the same inputs give the same answers.
//!
//! - [`machine`]: reading a declared machine against every rule, each refusal named by a
//!   stable id, and the views of a checked machine; [`selection`] says what a run is asked to
//!   do.
//! - [`hashes`]: the set hash, the machine hash, the run hash and the build-inputs hash.
//! - [`set_document`] and [`store`]: the stored set and the keys of a generation;
//!   [`verify`] checks a member and the keys of a generation over an in-memory tree.
//! - [`publish`], [`supersede`] and [`rows`]: the decision to put one coordinate, the
//!   decision to stand down for a newer build, and the verdict of a validation's rows.
//! - [`codec`] and [`transitions`]: the records a ledger holds, and which events move a record
//!   from which state to which.
//!
//! This crate is pure and synchronous: it holds no I/O, no wall clock and no `HashMap`.

pub mod codec;
pub mod hashes;
pub mod machine;
mod oci;
pub mod publish;
pub mod rows;
pub mod selection;
pub mod set_document;
pub mod store;
pub mod supersede;
pub mod transitions;
pub mod verify;
