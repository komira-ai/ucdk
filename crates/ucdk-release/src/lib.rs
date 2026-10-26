// SPDX-License-Identifier: Apache-2.0
//! The release model: the rules of a release that need no clock, no store and no registry, so
//! the same inputs give the same answers.
//!
//! - [`machine`]: reading a declared machine against every rule, each refusal named by a
//!   stable id, and the views of a checked machine.
//! - [`flags`]: the text rules of the flags and environment a machine authors for its build
//!   system, which the machine rules apply to every layer.
//! - [`selection`]: what a run is asked to do.
//!
//! This crate is pure and synchronous: it holds no I/O, no wall clock and no `HashMap`.

pub mod flags;
pub mod machine;
pub mod selection;
