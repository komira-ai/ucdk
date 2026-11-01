// SPDX-License-Identifier: Apache-2.0
//! The exit table: every class has the code the command line has always exited with.

use ucdk_api::v1::ExitClass;

#[test]
fn each_class_has_the_exit_code_the_command_line_documents() {
    let table = [
        (ExitClass::Done, 0),
        (ExitClass::Usage, 2),
        (ExitClass::Refused, 3),
        (ExitClass::FailedNoEffect, 4),
        (ExitClass::Indeterminate, 5),
        (ExitClass::FailedChanged, 6),
        (ExitClass::ValidationFailed, 7),
        (ExitClass::Leaks, 8),
    ];
    for (class, code) in table {
        assert_eq!(class.code(), code, "{class:?}");
        // The wire number is the exit code, so an SDK that reads either gets the same value.
        assert_eq!(class as i32, i32::from(code), "{class:?}");
    }
}

#[test]
fn the_table_names_every_class_the_wire_defines_and_no_two_share_a_code() {
    let defined: Vec<ExitClass> = (0..64)
        .filter_map(|n| ExitClass::try_from(n).ok())
        .collect();
    assert_eq!(defined.len(), 8, "{defined:?}");
    let mut codes: Vec<u8> = defined.iter().map(|c| c.code()).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), defined.len());
}

#[test]
fn only_usage_refused_and_failed_with_no_effect_promise_that_nothing_landed() {
    for class in (0..64).filter_map(|n| ExitClass::try_from(n).ok()) {
        let promises = matches!(
            class,
            ExitClass::Usage | ExitClass::Refused | ExitClass::FailedNoEffect
        );
        assert_eq!(class.promises_no_effect(), promises, "{class:?}");
    }
}
