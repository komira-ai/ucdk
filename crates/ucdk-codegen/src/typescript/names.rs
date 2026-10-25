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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_becomes_camel_case_and_pascal_case_for_method_names() {
        assert_eq!(camel("plan_hash"), "planHash");
        assert_eq!(camel("a_2b"), "a2b");
        assert_eq!(camel("a__b"), "aB");
        assert_eq!(camel("plain"), "plain");
        assert_eq!(camel(""), "");
        assert_eq!(pascal("which_choice"), "WhichChoice");
        assert_eq!(pascal("choice"), "Choice");
        assert_eq!(pascal(""), "");
    }

    #[test]
    fn a_member_a_class_keeps_gets_an_underscore() {
        for kept in [
            "encode",
            "decode",
            "encodeInto",
            "mergeFrom",
            "constructor",
            "toString",
            "valueOf",
        ] {
            assert_eq!(member(kept), format!("{kept}_"));
        }
        assert_eq!(member("encode_into"), "encodeInto_");
        assert_eq!(member("from"), "from");
    }

    #[test]
    fn a_parameter_that_is_a_word_or_a_name_the_generated_code_uses_gets_an_underscore() {
        for word in [
            "delete",
            "default",
            "new",
            "class",
            "fields",
            "options",
            "init",
            "arguments",
        ] {
            assert_eq!(param(word), format!("{word}_"));
        }
        assert_eq!(param("name"), "name");
        assert_eq!(param("from"), "from");
    }

    #[test]
    fn a_pair_is_written_short_when_both_sides_are_one_word() {
        assert_eq!(pair("a", "a"), "a");
        assert_eq!(pair("delete", "delete_"), "delete: delete_");
    }

    #[test]
    fn a_doc_comment_is_one_line_when_it_is_one_and_cannot_end_itself() {
        assert_eq!(doc("One.", 2), ["  /** One. */"]);
        assert_eq!(doc("A\n\nB", 0), ["/**", " * A", " *", " * B", " */"]);
        assert_eq!(doc("ends */ here", 0), ["/** ends *\\/ here */"]);
    }
}
