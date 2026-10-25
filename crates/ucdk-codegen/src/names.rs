// SPDX-License-Identifier: Apache-2.0
//! Names in the generated code: Python identifiers from proto names.

/// Words a Python identifier may not be, and the names the generated base class keeps for its own
/// methods. A proto name equal to one gets a trailing underscore.
const RESERVED: [&str; 37] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue",
    "def", "del", "elif", "else", "except", "finally", "for", "from", "global", "if", "import",
    "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield", "encode", "decode",
];

/// `name`, or `name_` when it is reserved.
pub fn ident(name: &str) -> String {
    if RESERVED.contains(&name) {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}

/// `ExitClass` as `EXIT_CLASS`.
fn screaming(name: &str) -> String {
    let mut out = String::new();
    let mut before = ' ';
    for c in name.chars() {
        if c.is_ascii_uppercase() && (before.is_ascii_lowercase() || before.is_ascii_digit()) {
            out.push('_');
        }
        out.push(c.to_ascii_uppercase());
        before = c;
    }
    out
}

/// The Python member name of an enum value: the proto name without the enum's own name as a
/// prefix (`EXIT_CLASS_DONE` of `ExitClass` is `DONE`), unless what is left is not a name.
pub fn enum_member(enum_name: &str, value: &str) -> String {
    let prefix = format!("{}_", screaming(enum_name));
    match value.strip_prefix(&prefix) {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_alphabetic()) => rest.to_owned(),
        _ => value.to_owned(),
    }
}

/// The module a proto package is written to: `ucdk.api.v1` is `ucdk_api_v1`.
pub fn module_of(package: &str) -> String {
    package.replace('.', "_")
}

/// The Python name of a type: its name inside its package, nested types joined by `_`
/// (`ucdk.resource.v1.Service.Public` is `Service_Public`).
pub fn local_name(full_name: &str, package: &str) -> String {
    let inside = full_name
        .strip_prefix(package)
        .unwrap_or(full_name)
        .trim_start_matches('.');
    inside.replace('.', "_")
}

/// The lines of a docstring body for `text`, indented by `indent` spaces; the quotes are the
/// caller's. Backslashes and triple quotes are escaped.
pub fn docstring(text: &str, indent: usize) -> Vec<String> {
    let pad = " ".repeat(indent);
    text.replace('\\', "\\\\")
        .replace("\"\"\"", "\\\"\\\"\\\"")
        .lines()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect()
}

/// A type named `full_name` of `package`, as written in a module of package `current`: bare in
/// its own module, qualified by the module it is imported from otherwise.
pub fn type_ref(package: &str, full_name: &str, current: &str) -> String {
    let name = local_name(full_name, package);
    if package == current {
        name
    } else {
        format!("{}.{name}", module_of(package))
    }
}
