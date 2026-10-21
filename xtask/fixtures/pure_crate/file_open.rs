// SPDX-License-Identifier: Apache-2.0
pub fn planted() -> bool {
    std::fs::File::open("input").is_ok()
}
