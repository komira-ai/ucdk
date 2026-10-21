// SPDX-License-Identifier: Apache-2.0
pub fn planted() -> bool {
    std::fs::OpenOptions::new().read(true).open("input").is_ok()
}
