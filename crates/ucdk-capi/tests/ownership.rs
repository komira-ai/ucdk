// SPDX-License-Identifier: Apache-2.0
#![allow(unsafe_code)]
//! Who owns what: a response is memory the library allocated and `ucdk_buf_free` releases, and a
//! loop of calls leaks nothing. This binary has one test and a counting allocator, so nothing
//! else allocates while it measures.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use std::collections::BTreeSet;

use common::{Scratch, call, file_config, open};
use prost::Message as _;
use ucdk_api::v1::LedgerShowRequest;
use ucdk_capi::{OK, Session, into_handle, ucdk_buf_free, ucdk_ledger_show, ucdk_session_close};
use ucdk_core::{RecordPath, Tag};
use ucdk_ledger::{Change, Ledger as _, MemoryLedger, Record};

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

fn live() -> isize {
    LIVE.load(Ordering::SeqCst)
}

// SAFETY: every method forwards to the system allocator with the arguments it was given and
// only adds to a counter, so the allocator contract is the system allocator's.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::SeqCst);
        // SAFETY: the caller's layout, passed on unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::SeqCst);
        // SAFETY: the caller's pointer and layout, passed on unchanged.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(new_size as isize - layout.size() as isize, Ordering::SeqCst);
        // SAFETY: the caller's pointer, layout and size, passed on unchanged.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Proves: a response is held by the library's allocation until `ucdk_buf_free` (the live bytes
/// rise by at least the response's length and fall back to exactly where they were), and 400
/// calls of `ucdk_ledger_show`, answering a document and answering the ledger error, each freed,
/// leave the live bytes where the first round left them. Catches a response that is copied and
/// the original leaked, a free that does not free, and a per-call leak.
#[test]
fn every_response_is_freed_by_buf_free_and_nothing_leaks() {
    // A ledger with records, so the answer has something in it.
    let ledger = MemoryLedger::new();
    for name in ["app/db", "app/run", "web/run"] {
        let token = ledger.read().expect("read").token;
        let record = Record::intent(
            RecordPath::new(name).expect("a path"),
            Tag::new("service"),
            "a name",
            BTreeSet::new(),
        );
        ledger.write(&token, Change::Put(record)).expect("write");
    }
    let document = into_handle(Session::with_ledger(Box::new(ledger)));
    // A file that is not a ledger, so the answer is the error.
    let scratch = Scratch::new("ownership");
    std::fs::write(scratch.file(), "not a ledger\n").expect("write");
    let broken = open(&file_config(&scratch.file())).session;

    let request = LedgerShowRequest {}.encode_to_vec();
    let round = || {
        for session in [document, broken] {
            assert_eq!(call(ucdk_ledger_show, session, &request).status, OK);
        }
    };
    round();
    let before = live();

    // One response, held: the library's allocation is what the live bytes show.
    let mut out = common::junk();
    // SAFETY: the request is readable for its length and `out` is a writable buffer.
    let status =
        unsafe { ucdk_ledger_show(document, request.as_ptr(), request.len(), &raw mut out) };
    assert_eq!(status, OK);
    assert!(live() - before >= isize::try_from(out.len).expect("fits"));
    // SAFETY: `out` is as the library gave it out and is freed once.
    unsafe { ucdk_buf_free(out) };
    assert_eq!(live(), before, "the free returned every byte");

    for _ in 0..200 {
        round();
    }
    assert_eq!(live(), before, "400 calls left nothing behind");
    ucdk_session_close(document);
    ucdk_session_close(broken);
}
