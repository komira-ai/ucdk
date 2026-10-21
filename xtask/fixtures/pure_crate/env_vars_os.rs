// SPDX-License-Identifier: Apache-2.0
pub fn planted() -> bool {
    std::env::vars_os().count() > 0
}
