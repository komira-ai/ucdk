// SPDX-License-Identifier: Apache-2.0
pub fn planted() -> bool {
    std::process::Command::new("true").status().is_ok()
}
