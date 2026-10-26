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
        (ExitClass::FailedChanged, 6),
        (ExitClass::Leaks, 8),
    ];
    for (class, code) in table {
        assert_eq!(class.code(), code, "{class:?}");
        // The wire number is the exit code, so an SDK that reads either gets the same value.
        assert_eq!(class as i32, i32::from(code), "{class:?}");
    }
}
