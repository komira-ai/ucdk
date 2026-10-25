// SPDX-License-Identifier: Apache-2.0
//! The TypeScript each field type becomes, and the wire functions that write and read it.
//!
//! 32 bit integers are `number`s, 64 bit integers are `bigint`s (a `number` cannot hold them
//! all), and an enum is a constant object whose type is the union of its values.

use prost_reflect::EnumDescriptor;

use crate::model::Scalar;
use crate::names::{enum_member, type_ref};

/// The TypeScript type, written as seen from `current`, the package of the module it is in.
pub fn ty(scalar: &Scalar, current: &str) -> String {
    match scalar {
        Scalar::Str => "string".to_owned(),
        Scalar::Bool => "boolean".to_owned(),
        Scalar::Int32 | Scalar::Uint32 => "number".to_owned(),
        Scalar::Int64 | Scalar::Uint64 => "bigint".to_owned(),
        Scalar::Enum(e) => type_ref(e.package_name(), e.full_name(), current),
        Scalar::Message(m) => type_ref(m.package_name(), m.full_name(), current),
    }
}

/// The value of a field of this type that was not written; a message is a new empty one.
pub fn default_value(scalar: &Scalar, current: &str) -> String {
    match scalar {
        Scalar::Str => "\"\"".to_owned(),
        Scalar::Bool => "false".to_owned(),
        Scalar::Int32 | Scalar::Uint32 => "0".to_owned(),
        Scalar::Int64 | Scalar::Uint64 => "0n".to_owned(),
        Scalar::Enum(e) => member_ref(e, e.default_value().name(), current),
        Scalar::Message(_) => format!("new {}()", ty(scalar, current)),
    }
}

/// The condition that `expr`, a value of this type, is not the default (so it is written).
pub fn is_set(scalar: &Scalar, expr: &str) -> String {
    match scalar {
        Scalar::Str => format!("{expr} !== \"\""),
        Scalar::Bool => expr.to_owned(),
        Scalar::Int32 | Scalar::Uint32 | Scalar::Enum(_) => format!("{expr} !== 0"),
        Scalar::Int64 | Scalar::Uint64 => format!("{expr} !== 0n"),
        Scalar::Message(_) => format!("{expr}.encode().length > 0"),
    }
}

/// The statement that appends field `number` holding `expr` to the `out` buffer.
pub fn put(scalar: &Scalar, out: &str, number: u32, expr: &str) -> String {
    let function = match scalar {
        Scalar::Str => "putString",
        Scalar::Bool => "putBool",
        Scalar::Message(_) => "putMessage",
        _ => "putVarintField",
    };
    format!("wire.{function}({out}, {number}, {expr});")
}

/// The expression that reads `raw`, the `WireValue` of a field, as this type.
pub fn get(scalar: &Scalar, current: &str, raw: &str) -> String {
    match scalar {
        Scalar::Str => format!("wire.str({raw})"),
        Scalar::Bool => format!("wire.bool({raw})"),
        Scalar::Int32 => format!("wire.int32({raw})"),
        Scalar::Int64 => format!("wire.int64({raw})"),
        Scalar::Uint32 => format!("wire.uint32({raw})"),
        Scalar::Uint64 => format!("wire.uint64({raw})"),
        Scalar::Enum(_) => format!("wire.enumValue<{}>({raw})", ty(scalar, current)),
        Scalar::Message(_) => format!("{}.decode(wire.bytesOf({raw}))", ty(scalar, current)),
    }
}

/// The name of the member of `e` called `value` in the proto.
pub fn member_name(e: &EnumDescriptor, value: &str) -> String {
    enum_member(e.name(), value)
}

/// `Enum.MEMBER`, as written in a module of package `current`.
pub fn member_ref(e: &EnumDescriptor, value: &str, current: &str) -> String {
    format!(
        "{}.{}",
        type_ref(e.package_name(), e.full_name(), current),
        member_name(e, value)
    )
}
