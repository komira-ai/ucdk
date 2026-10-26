// SPDX-License-Identifier: Apache-2.0
//! The library contains a panic with `catch_unwind` (architecture 5.3), which only works when a
//! panic unwinds. A build with `panic = "abort"` would turn every contained panic into a process
//! abort in the host, so the crate refuses to compile under it.

/// Proves: the crate source refuses to build under `panic = "abort"`, and this build unwinds.
/// Catches a removed guard: without it a profile that sets `panic = "abort"` would build a
/// library whose panic containment is silently gone.
#[test]
fn the_crate_refuses_to_build_where_a_panic_aborts() {
    let lib = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("the crate root");
    let compact: String = lib.split_whitespace().collect();
    assert!(
        compact.contains("#[cfg(panic=\"abort\")]compile_error!("),
        "ucdk-capi must carry a compile_error! under cfg(panic = \"abort\")"
    );
    // And a panic here is caught: this build unwinds.
    std::panic::set_hook(Box::new(|_| {}));
    let caught = std::panic::catch_unwind(|| panic!("contained"));
    assert!(caught.is_err());
}
