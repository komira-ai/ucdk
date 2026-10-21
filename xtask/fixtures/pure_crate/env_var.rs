// SPDX-License-Identifier: Apache-2.0
pub fn planted() -> bool {
    std::env::var("HOME").is_ok()
}
