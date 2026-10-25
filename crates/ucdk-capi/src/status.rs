// SPDX-License-Identifier: Apache-2.0
//! The status codes of the ABI. Each says whether the call itself worked; what the engine
//! decided is data in the response and always comes with [`OK`].

/// The call worked. `out` holds the response, which may itself say refused or failed.
pub const OK: i32 = 0;
/// Null where it is not allowed, a bad length, or a handle that is not an open session.
pub const INVALID_ARGUMENT: i32 = 1;
/// The request bytes are not the expected protobuf message.
pub const DECODE: i32 = 2;
/// The session is in use by another call.
pub const BUSY: i32 = 3;
/// The session config asks for something this library version does not have.
pub const ABI: i32 = 4;
/// A request longer than [`crate::MAX_REQUEST_LEN`].
pub const TOO_LARGE: i32 = 5;
/// A panic was contained; the session is poisoned and can only be closed.
pub const PANIC: i32 = 100;
/// A condition the library checks failed.
pub const INTERNAL: i32 = 101;

/// Why a call did not work: a status and the text of the `Error` message that goes with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallError {
    /// One of the codes above, never [`OK`].
    pub status: i32,
    /// For a person; never parsed.
    pub message: String,
}

impl CallError {
    pub(crate) fn new(status: i32, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}
