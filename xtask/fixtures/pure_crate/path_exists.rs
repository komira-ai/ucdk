// SPDX-License-Identifier: Apache-2.0
pub fn planted() -> bool {
    std::path::Path::new("input").exists()
}
