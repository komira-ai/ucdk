// SPDX-License-Identifier: Apache-2.0
//! The stable text id of each [`Reason`](crate::v1::Reason).

use crate::v1::Reason;

impl Reason {
    /// The id of the reason: `UCDK-E-` and the words of its name joined by `-`. A driver may
    /// branch on an id; the message next to it is for people and is never parsed.
    ///
    /// Each id is written out, so renaming a variant never changes the id a driver reads. The
    /// match has no wildcard, so a new reason does not compile until it has an id.
    pub fn id(self) -> &'static str {
        match self {
            Self::Unspecified => "UCDK-E-UNSPECIFIED",
            Self::MachineFormat => "UCDK-E-MACHINE-FORMAT",
            Self::MachineVersion => "UCDK-E-MACHINE-VERSION",
            Self::StageUnknown => "UCDK-E-STAGE-UNKNOWN",
            Self::Selector => "UCDK-E-SELECTOR",
            Self::SelectorNoMatch => "UCDK-E-SELECTOR-NO-MATCH",
            Self::SetMember => "UCDK-E-SET-MEMBER",
            Self::SetHash => "UCDK-E-SET-HASH",
            Self::Channel => "UCDK-E-CHANNEL",
            Self::Credential => "UCDK-E-CREDENTIAL",
            Self::ApprovedNames => "UCDK-E-APPROVED-NAMES",
            Self::PublishDifferentBytes => "UCDK-E-PUBLISH-DIFFERENT-BYTES",
            Self::PublishYanked => "UCDK-E-PUBLISH-YANKED",
            Self::PublishReadBack => "UCDK-E-PUBLISH-READ-BACK",
            Self::PublishUpload => "UCDK-E-PUBLISH-UPLOAD",
            Self::NotOnMainline => "UCDK-E-NOT-ON-MAINLINE",
            Self::ExceptionalReason => "UCDK-E-EXCEPTIONAL-REASON",
            Self::SupersededByNewer => "UCDK-E-SUPERSEDED-BY-NEWER",
            Self::InputsChanged => "UCDK-E-INPUTS-CHANGED",
            Self::Validation => "UCDK-E-VALIDATION",
            Self::CannotTell => "UCDK-E-CANNOT-TELL",
            Self::LeaseHeld => "UCDK-E-LEASE-HELD",
            Self::LeaseLost => "UCDK-E-LEASE-LOST",
            Self::Ledger => "UCDK-E-LEDGER",
            Self::BuildFailed => "UCDK-E-BUILD-FAILED",
            Self::RowAccounting => "UCDK-E-ROW-ACCOUNTING",
            Self::PinMismatch => "UCDK-E-PIN-MISMATCH",
            Self::Source => "UCDK-E-SOURCE",
            Self::StageNotAdmitted => "UCDK-E-STAGE-NOT-ADMITTED",
            Self::RunHashMismatch => "UCDK-E-RUN-HASH-MISMATCH",
            Self::LeaseUnconfirmed => "UCDK-E-LEASE-UNCONFIRMED",
            Self::MemberVersion => "UCDK-E-MEMBER-VERSION",
            Self::PlatformMismatch => "UCDK-E-PLATFORM-MISMATCH",
            Self::BuildOutput => "UCDK-E-BUILD-OUTPUT",
            Self::PublishStage => "UCDK-E-PUBLISH-STAGE",
            Self::PublishVanished => "UCDK-E-PUBLISH-VANISHED",
            Self::PublishBurned => "UCDK-E-PUBLISH-BURNED",
        }
    }
}
