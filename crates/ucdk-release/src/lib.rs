// SPDX-License-Identifier: Apache-2.0
//! The release model: the rules of a release that need no clock, no store and no registry, so
//! the same inputs give the same answers.
//!
//! - [`machine`]: reading a declared machine against every rule, each refusal named by a
//!   stable id, and the views of a checked machine.
//! - [`flags`]: the text rules of the flags and environment a machine authors for its build
//!   system, which the machine rules apply to every layer.
//! - [`selection`]: what a run is asked to do.
//! - [`hashes`] and [`action_input`]: the set hash, the machine hash, the run hash, the
//!   build-inputs hash and the action input digest, with the byte layout each is fed in.
//! - [`set_document`] and [`store`]: the stored set and the keys of a generation;
//!   [`verify`] checks a member and the keys of a generation over an in-memory tree, reading
//!   an OCI image layout where the member is an image.
//! - [`publish`] and [`supersede`]: the decision to put one coordinate, from what a registry
//!   reports of it, and the decision to stand down for a newer build.
//! - [`codec`] and [`transitions`]: the records a ledger holds, and which events move a record
//!   of each kind from which state to which.
//!
//! This crate is pure and synchronous: it holds no I/O, no wall clock and no `HashMap`.

pub mod action_input;
pub mod codec;
pub mod flags;
pub mod hashes;
pub mod machine;
mod oci;
pub mod publish;
pub mod selection;
pub mod set_document;
pub mod store;
pub mod supersede;
pub mod transitions;
pub mod verify;
