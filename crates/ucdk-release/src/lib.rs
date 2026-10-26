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
//!
//! - [`invocation`]: the argument vector and environment of one call of the build system,
//!   layered from the machine's base flags and environment, the target's and the flags ucdk
//!   owns; [`listing`] and [`test_report`] read what the build system printed and wrote.
//! - [`manifest`], [`closure`] and [`rows`]: the manifest the engine composes for a member, the
//!   `SET_CLOSURE` validation of a set, and the accounting of evaluated rows against derived
//!   ones.
//!
//! This crate is pure and synchronous: it holds no I/O, no wall clock and no `HashMap`.

pub mod action_input;
pub mod closure;
pub mod flags;
pub mod hashes;
pub mod invocation;
pub mod listing;
pub mod machine;
pub mod manifest;
mod oci;
pub mod rows;
pub mod selection;
pub mod set_document;
pub mod store;
pub mod test_report;
pub mod verify;
