// SPDX-License-Identifier: Apache-2.0
#![allow(unsafe_code)]
//! The header and the library say the same thing: the same ten functions in the same
//! order with the same signatures, the same constants, the same buffer layout, and every name
//! binds to the function of that name.

use std::collections::BTreeMap;
use std::fs;
use std::mem::{offset_of, size_of};
use std::path::Path;

use ucdk_capi::{
    ABI, ABI_MAJOR, ABI_MINOR, BUSY, Buf, DECODE, INTERNAL, INVALID_ARGUMENT, MAX_REQUEST_LEN, OK,
    PANIC, SYMBOLS, SessionHandle, TOO_LARGE,
};

fn header() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("include/ucdk.h"))
        .expect("the header is checked in")
}

/// The header without `/* ... */` comments.
fn without_comments(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        let end = rest[start..].find("*/").expect("a comment is closed") + start + 2;
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The header's function declarations, whitespace-normalised, in the order written.
fn prototypes(text: &str) -> Vec<String> {
    let stripped = without_comments(text);
    let code: Vec<&str> = stripped
        .lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .collect();
    code.join(" ")
        .split(';')
        .filter(|chunk| chunk.contains('('))
        .map(|chunk| {
            chunk
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .replace("( ", "(")
                .replace(" )", ")")
        })
        .collect()
}

fn defines(text: &str) -> BTreeMap<String, String> {
    without_comments(text)
        .lines()
        .filter_map(|l| l.trim().strip_prefix("#define "))
        .filter_map(|l| l.split_once(' '))
        .map(|(name, value)| (name.to_owned(), value.trim().to_owned()))
        .collect()
}

const PROTOTYPES: [&str; 10] = [
    "uint32_t ucdk_abi_version(void)",
    "int32_t ucdk_session_open(const uint8_t *config, size_t config_len, ucdk_session **out_session, ucdk_buf *out_error)",
    "void ucdk_session_close(ucdk_session *session)",
    "int32_t ucdk_load_catalog(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out)",
    "int32_t ucdk_plan(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out)",
    "int32_t ucdk_apply(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out)",
    "int32_t ucdk_destroy(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out)",
    "int32_t ucdk_status(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out)",
    "int32_t ucdk_leaks(ucdk_session *session, const uint8_t *req, size_t req_len, ucdk_buf *out)",
    "void ucdk_buf_free(ucdk_buf buf)",
];

/// The same ten, bound by name: a declaration the way a C caller's linker sees it. A name that
/// the library does not export, or a signature that differs from the Rust function's, fails
/// to link or to compile.
mod linked {
    use ucdk_capi::{Buf, SessionHandle};

    unsafe extern "C" {
        pub fn ucdk_abi_version() -> u32;
        pub fn ucdk_session_open(
            config: *const u8,
            config_len: usize,
            out_session: *mut *mut SessionHandle,
            out_error: *mut Buf,
        ) -> i32;
        pub fn ucdk_session_close(session: *mut SessionHandle);
        pub fn ucdk_load_catalog(s: *mut SessionHandle, r: *const u8, n: usize, o: *mut Buf)
        -> i32;
        pub fn ucdk_plan(s: *mut SessionHandle, r: *const u8, n: usize, o: *mut Buf) -> i32;
        pub fn ucdk_apply(s: *mut SessionHandle, r: *const u8, n: usize, o: *mut Buf) -> i32;
        pub fn ucdk_destroy(s: *mut SessionHandle, r: *const u8, n: usize, o: *mut Buf) -> i32;
        pub fn ucdk_status(s: *mut SessionHandle, r: *const u8, n: usize, o: *mut Buf) -> i32;
        pub fn ucdk_leaks(s: *mut SessionHandle, r: *const u8, n: usize, o: *mut Buf) -> i32;
        pub fn ucdk_buf_free(buf: Buf);
    }
}

/// Proves: the header declares exactly the functions the library exports, in the library's order
/// and with the signatures written out here, and declares nothing else. Catches a function added
/// to one and not the other, a changed parameter, or a stray declaration.
#[test]
fn the_header_declares_the_exported_functions() {
    let found = prototypes(&header());
    assert_eq!(found, PROTOTYPES);
    let names: Vec<&str> = found
        .iter()
        .map(|p| {
            p.split('(')
                .next()
                .and_then(|head| head.rsplit([' ', '*']).next())
                .expect("a name")
        })
        .collect();
    assert_eq!(names, SYMBOLS);
}

/// Proves: each symbol name binds to the Rust function of that name, with the signature the
/// header's table gives it: the declarations in `linked` resolve against the library by name,
/// and every address equals the function's own. Catches a function that lost its `no_mangle`, a
/// rename, and a signature that no longer matches the declaration.
#[test]
fn every_symbol_binds_to_its_function() {
    use ucdk_capi as lib;
    macro_rules! bound {
        ($($name:ident),*) => {
            [$((stringify!($name), linked::$name as *const (), lib::$name as *const ())),*]
        };
    }
    let pairs = bound!(
        ucdk_abi_version,
        ucdk_session_open,
        ucdk_session_close,
        ucdk_load_catalog,
        ucdk_plan,
        ucdk_apply,
        ucdk_destroy,
        ucdk_status,
        ucdk_leaks,
        ucdk_buf_free
    );
    for ((name, linked, own), symbol) in pairs.iter().zip(SYMBOLS) {
        assert_eq!(*name, symbol);
        assert_eq!(linked, own, "{name}");
    }
    // SAFETY: the function takes nothing and reads nothing.
    let version = unsafe { linked::ucdk_abi_version() };
    assert_eq!(version, lib::ucdk_abi_version());
}

/// Proves: every constant the header defines equals the library's, and the library has no
/// status the header lacks. Catches a code renumbered on one side only.
#[test]
fn the_header_constants_are_the_librarys() {
    let found = defines(&header());
    let expected: [(&str, i64); 11] = [
        ("UCDK_ABI_MAJOR", i64::from(ABI_MAJOR)),
        ("UCDK_ABI_MINOR", i64::from(ABI_MINOR)),
        (
            "UCDK_MAX_REQUEST_LEN",
            i64::try_from(MAX_REQUEST_LEN).expect("fits"),
        ),
        ("UCDK_OK", i64::from(OK)),
        ("UCDK_INVALID_ARGUMENT", i64::from(INVALID_ARGUMENT)),
        ("UCDK_DECODE", i64::from(DECODE)),
        ("UCDK_BUSY", i64::from(BUSY)),
        ("UCDK_ABI", i64::from(ABI)),
        ("UCDK_TOO_LARGE", i64::from(TOO_LARGE)),
        ("UCDK_PANIC", i64::from(PANIC)),
        ("UCDK_INTERNAL", i64::from(INTERNAL)),
    ];
    for (name, value) in expected {
        assert_eq!(
            found.get(name).map(String::as_str),
            Some(value.to_string().as_str()),
            "{name}"
        );
    }
    assert_eq!(
        found.len(),
        expected.len(),
        "the header defines nothing else: {found:?}"
    );
}

/// Proves: `ucdk_buf` is a pointer then a length, as the header lays it out, and the session
/// handle is a zero-sized opaque type. Catches a reordered or padded buffer.
#[test]
fn the_buffer_has_the_layout_the_header_declares() {
    assert_eq!(offset_of!(Buf, ptr), 0);
    assert_eq!(offset_of!(Buf, len), size_of::<usize>());
    assert_eq!(size_of::<Buf>(), 2 * size_of::<usize>());
    assert_eq!(size_of::<SessionHandle>(), 0);
    let text = without_comments(&header())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("typedef struct ucdk_buf { uint8_t *ptr; size_t len; } ucdk_buf;"));
}
