// SPDX-License-Identifier: Apache-2.0
//! The release engine: Test actions and validations. This commit holds the shape of the API and
//! the tests that hold it; the engine itself follows.

mod engine;
mod outcome;

pub use engine::{Engine, Request, Run, Seams};
pub use outcome::Outcome;

/// What every row of a Test action asserts of its target.
pub const TEST_ASSERTION: &str = "the test target passes";

/// What every row of a read-back asserts of its coordinate.
pub const READBACK_ASSERTION: &str = "the channel serves the set's digest";

/// The file of a set document inside its entry of the store.
pub const SET_FILE: &str = "set.pb";
