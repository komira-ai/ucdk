// SPDX-License-Identifier: Apache-2.0
//! The one table that maps a ucdk platform to a mopm subdirectory and back. A bijection: a
//! subdirectory read from a record maps back to exactly one platform.

use std::fmt;

/// mopm's subdirectories, as the registry's `SUBDIRS_V1` lists them.
pub const SUBDIRS_V1: [&str; 4] = ["linux-64", "linux-aarch64", "noarch", "osx-arm64"];

/// A mopm subdirectory. The table is [`Subdir::platform`] and [`Subdir::as_str`]; both lookups
/// the other way search [`Subdir::ALL`], so there is no second copy to drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subdir {
    Linux64,
    LinuxAarch64,
    Noarch,
    OsxArm64,
}

impl Subdir {
    pub const ALL: [Subdir; 4] = [
        Subdir::Linux64,
        Subdir::LinuxAarch64,
        Subdir::Noarch,
        Subdir::OsxArm64,
    ];

    /// The name mopm uses on the wire and in `info/index.json`.
    pub fn as_str(self) -> &'static str {
        match self {
            Subdir::Linux64 => "linux-64",
            Subdir::LinuxAarch64 => "linux-aarch64",
            Subdir::Noarch => "noarch",
            Subdir::OsxArm64 => "osx-arm64",
        }
    }

    /// The ucdk platform of this subdirectory. `any` is `noarch`; the macOS spelling is the
    /// in-tree one (`darwin-aarch64`). `darwin-arm64` is in no row and so is refused.
    pub fn platform(self) -> &'static str {
        match self {
            Subdir::Linux64 => "linux-x86_64",
            Subdir::LinuxAarch64 => "linux-aarch64",
            Subdir::Noarch => "any",
            Subdir::OsxArm64 => "darwin-aarch64",
        }
    }

    /// The subdirectory of a ucdk platform; `None` for a platform mopm cannot hold.
    pub fn of_platform(platform: &str) -> Option<Subdir> {
        Subdir::ALL.into_iter().find(|s| s.platform() == platform)
    }

    /// The subdirectory a record names; `None` for one outside `SUBDIRS_V1`.
    pub fn parse(text: &str) -> Option<Subdir> {
        Subdir::ALL.into_iter().find(|s| s.as_str() == text)
    }
}

impl fmt::Display for Subdir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
