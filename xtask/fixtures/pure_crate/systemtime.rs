// SPDX-License-Identifier: Apache-2.0
pub fn planted() -> bool {
    std::time::SystemTime::now() > std::time::UNIX_EPOCH
}
