// SPDX-License-Identifier: Apache-2.0
pub fn planted() -> bool {
    std::time::SystemTime::UNIX_EPOCH.elapsed().is_ok()
}
