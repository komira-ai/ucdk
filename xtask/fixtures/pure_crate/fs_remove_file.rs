// SPDX-License-Identifier: Apache-2.0
pub fn planted() -> bool {
    std::fs::remove_file("input").is_ok()
}
