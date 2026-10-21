// SPDX-License-Identifier: Apache-2.0
pub fn planted() -> bool {
    std::fs::read_to_string("input").is_ok()
}
