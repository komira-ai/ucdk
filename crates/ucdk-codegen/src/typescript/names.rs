// SPDX-License-Identifier: Apache-2.0
//! Names in the generated TypeScript: lower camel case members from snake case proto names, the
//! few that need care, and the comments.

/// Names a message class keeps for its own methods (and the ones every object has). A proto
/// name equal to one gets a trailing underscore.
const MEMBERS: &[&str] = &[
    "encode",
    "decode",
    "encodeInto",
    "mergeFrom",
    "constructor",
    "toString",
    "valueOf",
];

/// Words that are not names of variables or parameters, and the names the generated code uses
/// for its own parameters.
const WORDS: &[&str] = &[
    "arguments",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "fields",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "options",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "undefined",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "init",
];

/// Members of a class constructor that a static method may not take the name of.
pub const STATIC_TAKEN: &[&str] = &["name", "length", "prototype", "caller", "arguments"];

/// `plan_hash` as `planHash`: each word after the first starts with a capital.
pub fn camel(name: &str) -> String {
    let mut out = String::new();
    for (i, word) in name.split('_').enumerate() {
        let mut chars = word.chars();
        match (i, chars.next()) {
            (0, Some(first)) => out.push(first),
            (_, Some(first)) => out.extend(first.to_uppercase()),
            (_, None) => {}
        }
        out.push_str(chars.as_str());
    }
    out
}

/// `choice` as `Choice`, for the names of the methods of a oneof.
pub fn pascal(name: &str) -> String {
    let name = camel(name);
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// The member name of a proto name: camel case, and a trailing underscore when a message class
/// keeps it.
pub fn member(name: &str) -> String {
    let name = camel(name);
    if MEMBERS.contains(&name.as_str()) {
        format!("{name}_")
    } else {
        name
    }
}

/// `name` as a variable or a parameter: a trailing underscore when it is not allowed to be one.
pub fn param(name: &str) -> String {
    if WORDS.contains(&name) {
        format!("{name}_")
    } else {
        name.to_owned()
    }
}

/// `name: value`, or just `name` when the value is the same word.
pub fn pair(name: &str, value: &str) -> String {
    if name == value {
        name.to_owned()
    } else {
        format!("{name}: {value}")
    }
}

/// A doc comment of `text`, indented by `indent` spaces: one line when it is one, else a block.
/// `*/` inside the text is broken so it cannot end the comment.
pub fn doc(text: &str, indent: usize) -> Vec<String> {
    let pad = " ".repeat(indent);
    let text = text.replace("*/", "*\\/");
    let lines: Vec<&str> = text.lines().collect();
    if let [line] = lines.as_slice() {
        return vec![format!("{pad}/** {line} */")];
    }
    let mut out = vec![format!("{pad}/**")];
    out.extend(
        lines
            .iter()
            .map(|l| format!("{pad} * {l}").trim_end().to_owned()),
    );
    out.push(format!("{pad} */"));
    out
}
