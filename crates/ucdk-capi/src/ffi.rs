// SPDX-License-Identifier: Apache-2.0
//! The functions a C caller links against, and the only `unsafe` in the workspace.
//!
//! The rules every function here follows:
//!
//! - **Nothing unwinds out.** The body of each runs inside `catch_unwind`; a caught panic is
//!   answered with `PANIC`. The panic poisons the session: the call held the session's lock, so
//!   every later call on it is refused (see [`Session::call`]).
//! - **A handle is a number, looked up and never trusted.** A `ucdk_session *` carries a number
//!   from a counter that only grows, and the library keeps the map from open numbers to sessions.
//!   A call finds its session there, so a handle that was closed, closed twice, or never opened is
//!   refused, even after the allocator has reused an address; nothing reads through the pointer.
//!   A call in flight keeps its session alive past a close.
//! - **The library allocates what it hands out.** A response is a boxed slice whose pointer and
//!   length go to the caller; [`ucdk_buf_free`] is the only way it is released.
//! - **Request bytes are borrowed.** The slice lives for the call; nothing keeps the pointer.
//! - **Outputs are zeroed first.** On any failure `*out` is empty or an `Error` message.

use std::any::Any;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::session::{Session, Verb, error_bytes};
use crate::status::{self, CallError};
use crate::{ABI_MAJOR, ABI_MINOR, MAX_REQUEST_LEN};

/// The opaque `ucdk_session` of the header.
#[repr(C)]
pub struct SessionHandle {
    _private: [u8; 0],
}

/// The `ucdk_buf` of the header: bytes the library allocated. An empty one is all zeros.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Buf {
    /// The first byte, or null when the buffer is empty.
    pub ptr: *mut u8,
    /// The number of bytes.
    pub len: usize,
}

impl Buf {
    /// No bytes; freeing it does nothing.
    pub const EMPTY: Self = Self {
        ptr: ptr::null_mut(),
        len: 0,
    };

    fn from_vec(bytes: Vec<u8>) -> Self {
        if bytes.is_empty() {
            return Self::EMPTY;
        }
        let boxed = bytes.into_boxed_slice();
        let len = boxed.len();
        Self {
            ptr: Box::into_raw(boxed).cast::<u8>(),
            len,
        }
    }
}

/// The sessions that are open, by the number their handle carries.
static OPEN: Mutex<BTreeMap<usize, Arc<Session>>> = Mutex::new(BTreeMap::new());

/// The number the next handle carries. It only grows, so a number is never handed out twice and
/// a handle that was closed stays refused whatever a later session or the allocator does.
static NEXT: AtomicUsize = AtomicUsize::new(1);

fn open_set() -> MutexGuard<'static, BTreeMap<usize, Arc<Session>>> {
    // The map is only inserted into and removed from, and no code of ours runs while it is
    // locked, so a poisoned lock still holds a whole map.
    OPEN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The handle as a pointer: a number, never an address, so nothing ever reads through it.
fn handle_of(number: usize) -> *mut SessionHandle {
    ptr::without_provenance_mut(number)
}

/// Registers `session` and returns the handle a C caller passes back; `ucdk_session_close`
/// releases it. This is what `ucdk_session_open` does after it has built the session, and what a
/// Rust host or a test with a ledger of its own calls.
pub fn into_handle(session: Session) -> *mut SessionHandle {
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    open_set().insert(number, Arc::new(session));
    handle_of(number)
}

/// The session behind `handle`, if it is open.
fn lookup(handle: *mut SessionHandle) -> Option<Arc<Session>> {
    open_set().get(&handle.addr()).cloned()
}

/// Writes `value` to `slot` unless it is null.
///
/// # Safety
///
/// `slot`, when not null, must be valid for a write of one `T`.
unsafe fn put<T>(slot: *mut T, value: T) {
    if !slot.is_null() {
        // SAFETY: not null, and valid for a write by the caller's contract. `write` does not
        // read or drop what was there, which may be uninitialised.
        unsafe { slot.write(value) };
    }
}

/// The request bytes, borrowed for the call.
///
/// # Safety
///
/// When `len` is not zero, `ptr` must be valid for reads of `len` bytes for the length of
/// the borrow.
unsafe fn borrow<'a>(ptr: *const u8, len: usize) -> Result<&'a [u8], CallError> {
    if len == 0 {
        return Ok(&[]);
    }
    if ptr.is_null() {
        return Err(CallError::new(
            status::INVALID_ARGUMENT,
            "a request with a length has no bytes",
        ));
    }
    if len > MAX_REQUEST_LEN {
        return Err(CallError::new(
            status::TOO_LARGE,
            format!("a request is at most {MAX_REQUEST_LEN} bytes"),
        ));
    }
    // SAFETY: not null, at most `MAX_REQUEST_LEN` (so it fits an `isize`), and valid for reads
    // of `len` bytes for the borrow by the caller's contract.
    Ok(unsafe { std::slice::from_raw_parts(ptr, len) })
}

fn describe(payload: &(dyn Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<&str>() {
        text
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text
    } else {
        "a panic with no text"
    }
}

/// A contained panic is a failed call.
fn contain<T>(result: std::thread::Result<Result<T, CallError>>) -> Result<T, CallError> {
    result.unwrap_or_else(|payload| {
        Err(CallError::new(
            status::PANIC,
            format!("panic contained: {}", describe(payload.as_ref())),
        ))
    })
}

/// Writes the `Error` message of a failed call to `out` and gives its status.
///
/// # Safety
///
/// `out`, when not null, must be valid for a write of one [`Buf`].
unsafe fn fail(error: &CallError, out: *mut Buf) -> i32 {
    // SAFETY: by the caller's contract.
    unsafe { put(out, Buf::from_vec(error_bytes(error))) };
    error.status
}

/// The version: `(major << 16) | minor`. Callable before anything else.
#[unsafe(no_mangle)]
pub extern "C" fn ucdk_abi_version() -> u32 {
    (ABI_MAJOR << 16) | ABI_MINOR
}

/// Opens a session from a `SessionConfig` message.
///
/// # Safety
///
/// `config` must be valid for reads of `config_len` bytes unless `config_len` is zero;
/// `out_session` must be valid for a write of a pointer; `out_error` must be null or valid
/// for a write of a [`Buf`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ucdk_session_open(
    config: *const u8,
    config_len: usize,
    out_session: *mut *mut SessionHandle,
    out_error: *mut Buf,
) -> i32 {
    // SAFETY: `out_error` is null or writable by the contract.
    unsafe { put(out_error, Buf::EMPTY) };
    // SAFETY: `out_session` is writable by the contract (`put` skips a null).
    unsafe { put(out_session, ptr::null_mut()) };
    if out_session.is_null() {
        let error = CallError::new(status::INVALID_ARGUMENT, "out_session is null");
        // SAFETY: `out_error` is null or writable by the contract.
        return unsafe { fail(&error, out_error) };
    }
    let opened = contain(catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: `config` is readable for `config_len` bytes by the contract.
        let bytes = unsafe { borrow(config, config_len) }?;
        Session::open(bytes)
    })));
    match opened {
        Ok(session) => {
            // SAFETY: `out_session` is writable and not null.
            unsafe { put(out_session, into_handle(session)) };
            status::OK
        }
        // SAFETY: `out_error` is null or writable by the contract.
        Err(error) => unsafe { fail(&error, out_error) },
    }
}

/// Closes a session. A null handle, one already closed and one never opened are ignored.
#[unsafe(no_mangle)]
pub extern "C" fn ucdk_session_close(session: *mut SessionHandle) {
    let _ = catch_unwind(|| {
        // Taken out of the map first, so the lock is not held while the session is dropped. A
        // call in flight holds a count of its own and drops the session itself.
        let closed = open_set().remove(&session.addr());
        drop(closed);
    });
}

/// The body of the verbs.
///
/// # Safety
///
/// The contract of the verbs: `req` readable for `req_len` bytes unless that is zero, `out`
/// valid for a write of a [`Buf`].
unsafe fn call_verb(
    session: *mut SessionHandle,
    req: *const u8,
    req_len: usize,
    out: *mut Buf,
    verb: Verb,
) -> i32 {
    if out.is_null() {
        return status::INVALID_ARGUMENT;
    }
    // SAFETY: not null, writable by the contract.
    unsafe { put(out, Buf::EMPTY) };
    let result = match lookup(session) {
        None => Err(CallError::new(
            status::INVALID_ARGUMENT,
            "the session is not open",
        )),
        // The session moves into the closure, so if a concurrent close left this call holding
        // the last count, the drop that follows runs inside `catch_unwind` too.
        Some(session) => contain(catch_unwind(AssertUnwindSafe(move || {
            // SAFETY: `req` is readable for `req_len` bytes by the contract.
            let bytes = unsafe { borrow(req, req_len) }?;
            session.call(verb, bytes)
        }))),
    };
    match result {
        Ok(bytes) => {
            // SAFETY: not null, writable by the contract.
            unsafe { put(out, Buf::from_vec(bytes)) };
            status::OK
        }
        // SAFETY: not null, writable by the contract.
        Err(error) => unsafe { fail(&error, out) },
    }
}

macro_rules! verb {
    ($(#[$doc:meta])* $name:ident, $verb:expr) => {
        $(#[$doc])*
        ///
        /// # Safety
        ///
        /// `req` must be valid for reads of `req_len` bytes unless `req_len` is zero, and `out`
        /// must be valid for a write of a [`Buf`].
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            session: *mut SessionHandle,
            req: *const u8,
            req_len: usize,
            out: *mut Buf,
        ) -> i32 {
            // SAFETY: the contract of this function is the contract of `call_verb`.
            unsafe { call_verb(session, req, req_len, out, $verb) }
        }
    };
}

verb!(
    /// Shows the session's ledger for a `LedgerShowRequest`; answers an `Outcome` (the
    /// `ledger_show` arm, or the `error` arm when the ledger cannot be opened or read). It takes
    /// no lease and changes no file: a file ledger is opened read-only for the call.
    ucdk_ledger_show,
    Verb::LedgerShow
);

/// Releases a buffer the library filled. A zeroed buffer is ignored.
///
/// # Safety
///
/// `buf` must be empty or exactly as the library gave it out, and not freed before.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ucdk_buf_free(buf: Buf) {
    let _ = catch_unwind(|| {
        if !buf.ptr.is_null() && buf.len != 0 {
            let slice = ptr::slice_from_raw_parts_mut(buf.ptr, buf.len);
            // SAFETY: by the contract `buf` is a pointer and length that `Buf::from_vec` took
            // from a `Box<[u8]>`, not freed since, so this rebuilds that box.
            drop(unsafe { Box::from_raw(slice) });
        }
    });
}

/// The functions are `#[unsafe(no_mangle)]`, so every test binary of this crate links its own
/// copy of each under one symbol name, and the coverage tool attributes the counts of one copy to
/// the source line. Which copy it takes follows the build graph, so the lines of a function that
/// only the integration tests call can read zero. This module calls every exported function from
/// inside the crate, so the copy in the crate's own test binary is exercised whichever copy is
/// chosen. The integration tests stay: they prove the ABI as a host sees it.
#[cfg(test)]
mod tests {
    use prost::Message as _;
    use ucdk_api::v1::{ErrorCode, LedgerShowRequest, SessionConfig};

    use super::*;

    fn bytes_of(buf: Buf) -> Vec<u8> {
        let bytes = if buf.ptr.is_null() {
            Vec::new()
        } else {
            // SAFETY: the library filled `buf` with `len` bytes at `ptr`, not freed since.
            unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec()
        };
        // SAFETY: `buf` is as the library gave it out (or empty), freed here once.
        unsafe { ucdk_buf_free(buf) };
        bytes
    }

    #[test]
    fn an_empty_answer_is_a_valid_empty_buffer() {
        assert_eq!(Buf::from_vec(Vec::new()), Buf::EMPTY);
        let one = Buf::from_vec(vec![7]);
        assert!(!one.ptr.is_null());
        assert_eq!(one.len, 1);
        assert_eq!(bytes_of(one), [7]);
        // SAFETY: an empty buffer is the documented one, and freeing it does nothing.
        unsafe { ucdk_buf_free(Buf::EMPTY) };
    }

    #[test]
    fn every_exported_function_is_called_from_inside_the_crate() {
        assert_eq!(ucdk_abi_version(), (ABI_MAJOR << 16) | ABI_MINOR);

        // The open: a good config, a null `out_session` and bytes that are not a config.
        let config = SessionConfig::default().encode_to_vec();
        let mut session = ptr::null_mut();
        let mut error = Buf::EMPTY;
        // SAFETY: `config` is readable for its length and both outputs are writable.
        let status = unsafe {
            ucdk_session_open(
                config.as_ptr(),
                config.len(),
                &raw mut session,
                &raw mut error,
            )
        };
        assert_eq!(status, status::OK);
        assert!(!session.is_null());
        assert!(bytes_of(error).is_empty());

        // SAFETY: the null `out_session` is under test; `out_error` is writable.
        let status = unsafe { ucdk_session_open(ptr::null(), 0, ptr::null_mut(), &raw mut error) };
        assert_eq!(status, status::INVALID_ARGUMENT);
        let message = ucdk_api::v1::Error::decode(bytes_of(error).as_slice()).expect("an error");
        assert_eq!(message.code, ErrorCode::InvalidRequest as i32);
        let mut other = ptr::null_mut();
        // SAFETY: bytes that are not a config; both outputs are writable.
        let status = unsafe {
            ucdk_session_open(
                [0xff, 0xff, 0xff].as_ptr(),
                3,
                &raw mut other,
                &raw mut error,
            )
        };
        assert_eq!((status, other.is_null()), (status::DECODE, true));
        bytes_of(error);

        // The verb: a good call, a null `out`, bytes that are not the request, and a handle
        // that is not open.
        let request = LedgerShowRequest {}.encode_to_vec();
        let mut out = Buf::EMPTY;
        // SAFETY: `request` is readable for its length and `out` is writable.
        let status =
            unsafe { ucdk_ledger_show(session, request.as_ptr(), request.len(), &raw mut out) };
        assert_eq!(status, status::OK);
        assert!(!bytes_of(out).is_empty());
        // SAFETY: the null `out` is under test; a zero length reads nothing.
        let status = unsafe { ucdk_ledger_show(session, ptr::null(), 0, ptr::null_mut()) };
        assert_eq!(status, status::INVALID_ARGUMENT);
        let mut out = Buf::EMPTY;
        // SAFETY: bytes that are not the request; `out` is writable.
        let status =
            unsafe { ucdk_ledger_show(session, [0xff, 0xff, 0xff].as_ptr(), 3, &raw mut out) };
        assert_eq!(status, status::DECODE);
        bytes_of(out);

        ucdk_session_close(session);
        let mut out = Buf::EMPTY;
        // SAFETY: `out` is writable; the handle is closed, which is under test.
        let status = unsafe { ucdk_ledger_show(session, ptr::null(), 0, &raw mut out) };
        assert_eq!(status, status::INVALID_ARGUMENT);
        bytes_of(out);
        // A second close, and a close of null, do nothing.
        ucdk_session_close(session);
        ucdk_session_close(ptr::null_mut());
    }
}
