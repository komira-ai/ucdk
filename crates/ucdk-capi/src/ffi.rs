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
/// Rust host or a test with a provider of its own calls.
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

/// The body of the five verbs.
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
    /// Validates and lowers a `ResourceList`; answers a `CatalogResponse`.
    ucdk_load_catalog,
    Verb::LoadCatalog
);
verb!(
    /// Plans a `PlanRequest`; answers an `Outcome`.
    ucdk_plan,
    Verb::Plan
);
verb!(
    /// Applies the plan an `ApplyRequest` pins by hash; answers an `Outcome`.
    ucdk_apply,
    Verb::Apply
);
verb!(
    /// Destroys by the plan a `DestroyRequest` pins by hash; answers an `Outcome`.
    ucdk_destroy,
    Verb::Destroy
);
verb!(
    /// Reads the ledger and the live objects for a `StatusRequest`; answers an `Outcome`.
    ucdk_status,
    Verb::Status
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
