// SPDX-License-Identifier: Apache-2.0
//! The text rules of the flags and environment a machine authors: the match of a flag element
//! against the flags ucdk owns, the placeholder scanner, and the environment-key grammar.

use ucdk_release::flags::{
    BraceError, EnvKeyError, Placeholder, Reserved, env_key, owned_flags, placeholders, reserved,
    substitute,
};
use ucdk_release_proto::v1::BuildSystemKind;
use ucdk_release_proto::v1::BuildSystemKind::{Bazel, Buck2, Unspecified};

fn refused(kind: BuildSystemKind, element: &str) -> bool {
    reserved(kind, element).is_some()
}

#[test]
fn the_owned_flags_are_the_closed_list_of_each_kind() {
    assert_eq!(
        owned_flags(Buck2),
        [
            "show-output",
            "show-full-output",
            "show-simple-output",
            "show-json-output",
            "show-full-json-output",
            "event-log"
        ]
    );
    assert_eq!(
        owned_flags(Bazel),
        [
            "output",
            "build_event_json_file",
            "build_event_text_file",
            "build_event_binary_file",
            "symlink_prefix"
        ]
    );
    assert!(owned_flags(Unspecified).is_empty());
}

#[test]
fn an_element_equal_to_a_listed_flag_is_refused() {
    for flag in owned_flags(Buck2) {
        assert_eq!(
            reserved(Buck2, &format!("--{flag}")),
            Some(Reserved::Owned(flag)),
            "{flag}"
        );
    }
    for flag in owned_flags(Bazel) {
        assert_eq!(
            reserved(Bazel, &format!("--{flag}")),
            Some(Reserved::Owned(flag)),
            "{flag}"
        );
    }
}

#[test]
fn a_listed_flag_with_an_equals_value_is_refused() {
    assert_eq!(
        reserved(Buck2, "--event-log=/tmp/events"),
        Some(Reserved::Owned("event-log"))
    );
    assert_eq!(
        reserved(Bazel, "--output=files"),
        Some(Reserved::Owned("output"))
    );
    // Only the first `=` ends the name, and an empty value is still a value.
    assert!(refused(Bazel, "--symlink_prefix="));
    assert!(refused(Bazel, "--output=a=b"));
    // The text after the `=` is never read as a name.
    assert!(!refused(Bazel, "--define=output=files"));
}

#[test]
fn a_negated_listed_flag_is_refused() {
    for element in ["--nooutput", "--no_output", "--no-output"] {
        assert_eq!(
            reserved(Bazel, element),
            Some(Reserved::Owned("output")),
            "{element}"
        );
    }
    for element in [
        "--noevent-log",
        "--no-event-log",
        "--no_event_log",
        "--noevent_log=x",
    ] {
        assert_eq!(
            reserved(Buck2, element),
            Some(Reserved::Owned("event-log")),
            "{element}"
        );
    }
    assert!(refused(Bazel, "--nobuild_event_json_file"));
    // `no` alone, or `no` followed by nothing, names nothing.
    assert!(!refused(Bazel, "--no"));
    assert!(!refused(Bazel, "--no_"));
}

#[test]
fn a_strict_prefix_of_a_listed_buck2_flag_is_refused() {
    for element in [
        "--event",
        "--event-",
        "--ev",
        "--show",
        "--show-full",
        "--e",
        "--s",
    ] {
        assert!(refused(Buck2, element), "{element}");
    }
    // A prefix, spelled negated, is a prefix of the flag as well.
    assert!(refused(Buck2, "--noevent"));
}

#[test]
fn a_strict_prefix_of_a_listed_bazel_flag_is_accepted() {
    for element in [
        "--build",
        "--nobuild",
        "--o",
        "--out",
        "--build_event",
        "--symlink",
    ] {
        assert!(!refused(Bazel, element), "{element}");
    }
}

#[test]
fn hyphen_and_underscore_spellings_match_alike() {
    assert!(refused(Buck2, "--event_log"));
    assert!(refused(Buck2, "--event-log"));
    assert!(refused(Buck2, "--show_full_output"));
    assert!(refused(Buck2, "--show-full_output"));
    assert!(refused(Bazel, "--build-event-json-file"));
    assert!(refused(Bazel, "--build_event-json_file=x"));
    assert!(refused(Bazel, "--symlink-prefix"));
}

#[test]
fn a_longer_name_that_only_starts_like_a_listed_flag_is_accepted() {
    assert!(!refused(Bazel, "--output_filter"));
    assert!(!refused(Bazel, "--output_filter=x"));
    assert!(!refused(Bazel, "--build_event_json_file_path_conversion"));
    assert!(!refused(Buck2, "--event-log-extra"));
    assert!(!refused(Buck2, "--show-output-format"));
}

#[test]
fn a_value_or_a_short_option_is_never_matched() {
    for element in [
        "files",
        "-c",
        "-e",
        "output",
        "event-log",
        "release.version=1",
        "-",
        "",
        "x=--output",
    ] {
        assert!(!refused(Bazel, element), "{element:?}");
        assert!(!refused(Buck2, element), "{element:?}");
    }
    // A bare `--name` with an empty name is not a flag either.
    assert!(!refused(Bazel, "--="));
}

#[test]
fn a_flag_listed_for_the_other_kind_is_accepted() {
    for flag in owned_flags(Bazel) {
        assert!(!refused(Buck2, &format!("--{flag}")), "{flag}");
    }
    for flag in owned_flags(Buck2) {
        assert!(!refused(Bazel, &format!("--{flag}")), "{flag}");
    }
}

#[test]
fn the_end_of_options_marker_is_refused_for_both_kinds() {
    assert_eq!(reserved(Buck2, "--"), Some(Reserved::EndOfOptions));
    assert_eq!(reserved(Bazel, "--"), Some(Reserved::EndOfOptions));
    assert_eq!(reserved(Unspecified, "--"), Some(Reserved::EndOfOptions));
    assert!(!refused(Bazel, "---"));
}

#[test]
fn a_buck2_argfile_element_is_refused_and_a_bazel_one_is_not() {
    assert_eq!(reserved(Buck2, "@args.txt"), Some(Reserved::ArgFile));
    assert_eq!(reserved(Buck2, "@"), Some(Reserved::ArgFile));
    assert!(!refused(Bazel, "@args.txt"));
    assert!(!refused(Buck2, "x@y"));
}

#[test]
fn a_kind_that_is_not_a_build_system_owns_nothing() {
    assert!(!refused(Unspecified, "--output=files"));
    assert!(!refused(Unspecified, "--event-log"));
}

/// The rows of the design's table: the element, whether Buck2 refuses it, whether Bazel does.
/// The table is held here as data so that the page and the function cannot drift.
const TABLE: [(&str, bool, bool); 20] = [
    ("--event-log=x", true, false),
    ("--event-log", true, false),
    ("--event_log", true, false),
    ("--no-event-log", true, false),
    ("--event", true, false),
    ("--output=files", false, true),
    ("--output", false, true),
    ("--nooutput", false, true),
    ("--no_output", false, true),
    ("--build_event_json_file=x", false, true),
    ("--build_event", false, false),
    ("--build", false, false),
    ("--nobuild", false, false),
    ("--output_filter=x", false, false),
    ("--build_event_json_file_path_conversion", false, false),
    ("--", true, true),
    ("@argfile", true, false),
    ("files", false, false),
    ("-c", false, false),
    ("release.version=1", false, false),
];

#[test]
fn the_match_table_of_the_design_is_the_function() {
    for (element, buck2, bazel) in TABLE {
        assert_eq!(refused(Buck2, element), buck2, "Buck2 {element}");
        assert_eq!(refused(Bazel, element), bazel, "Bazel {element}");
    }
}

#[test]
fn a_refusal_says_why_in_words() {
    let owned = reserved(Buck2, "--event-log").expect("refused").to_string();
    assert!(owned.contains("--event-log"), "{owned}");
    let end = reserved(Bazel, "--").expect("refused").to_string();
    assert!(end.contains("--"), "{end}");
    let file = reserved(Buck2, "@x").expect("refused").to_string();
    assert!(file.contains("argument file"), "{file}");
}

#[test]
fn the_placeholders_of_a_text_come_in_the_order_they_occur() {
    assert_eq!(placeholders("plain"), Ok(vec![]));
    assert_eq!(placeholders(""), Ok(vec![]));
    assert_eq!(
        placeholders("{revision}-{release_version}-{revision}"),
        Ok(vec![
            Placeholder::Revision,
            Placeholder::ReleaseVersion,
            Placeholder::Revision
        ])
    );
    let all: String = Placeholder::ALL
        .iter()
        .map(|p| format!("{{{}}}", p.name()))
        .collect();
    assert_eq!(placeholders(&all), Ok(Placeholder::ALL.to_vec()));
    // Doubled braces are literal braces, and a doubled brace next to a placeholder is read
    // from the left.
    assert_eq!(placeholders("{{revision}}"), Ok(vec![]));
    assert_eq!(
        placeholders("{{{revision}}}"),
        Ok(vec![Placeholder::Revision])
    );
    assert_eq!(
        placeholders("-c x={platform}\u{e9}"),
        Ok(vec![Placeholder::Platform])
    );
}

#[test]
fn the_placeholder_scanner_names_each_brace_it_refuses() {
    assert_eq!(
        placeholders("a{nope}b"),
        Err(BraceError::Unknown {
            at: 1,
            name: "nope".to_owned()
        })
    );
    assert_eq!(
        placeholders("{}"),
        Err(BraceError::Unknown {
            at: 0,
            name: String::new()
        })
    );
    assert_eq!(
        placeholders("ab{revision"),
        Err(BraceError::Unclosed { at: 2 })
    );
    assert_eq!(
        placeholders("{a{revision}"),
        Err(BraceError::Unclosed { at: 0 })
    );
    assert_eq!(placeholders("{"), Err(BraceError::Unclosed { at: 0 }));
    assert_eq!(placeholders("ab}"), Err(BraceError::Stray { at: 2 }));
    assert_eq!(placeholders("{{x}"), Err(BraceError::Stray { at: 3 }));
    // The first refusal is the one named.
    assert_eq!(placeholders("}{nope}"), Err(BraceError::Stray { at: 0 }));
    let unknown = placeholders("{nope}").expect_err("refused").to_string();
    assert!(
        unknown.contains("nope") && unknown.contains("release_version"),
        "{unknown}"
    );
    let open = placeholders("x{").expect_err("refused").to_string();
    assert!(open.contains("byte 1") && open.contains("{{"), "{open}");
    let stray = placeholders("x}").expect_err("refused").to_string();
    assert!(stray.contains("byte 1") && stray.contains("}}"), "{stray}");
}

#[test]
fn the_six_placeholders_are_the_closed_list() {
    let names: Vec<&str> = Placeholder::ALL.iter().map(|p| p.name()).collect();
    assert_eq!(
        names,
        [
            "release_version",
            "build_number",
            "source_commit",
            "revision",
            "timestamp_ms",
            "platform"
        ]
    );
}

#[test]
fn an_env_key_follows_the_grammar() {
    for key in [
        "PATH",
        "_X",
        "a",
        "A1",
        "http_proxy",
        "no_proxy",
        "UCDK",
        "UCD_X",
        "X_UCDK_",
    ] {
        assert_eq!(env_key(key), Ok(()), "{key}");
    }
    for key in ["", "1A", "A-B", "A B", "A=B", "A.B", "\u{e9}", "A\0"] {
        assert_eq!(env_key(key), Err(EnvKeyError::Grammar), "{key:?}");
    }
}

#[test]
fn a_ucdk_prefix_in_any_case_is_reserved() {
    for key in ["UCDK_X", "ucdk_x", "Ucdk_X", "uCdK_", "UCDK_"] {
        assert_eq!(env_key(key), Err(EnvKeyError::Reserved), "{key}");
    }
    assert!(EnvKeyError::Grammar.to_string().contains("[A-Za-z_]"));
    assert!(EnvKeyError::Reserved.to_string().contains("UCDK_"));
}

fn named(p: Placeholder) -> String {
    format!("<{}>", p.name())
}

#[test]
fn substitution_replaces_each_placeholder_and_unescapes_each_doubled_brace() {
    assert_eq!(substitute("plain", named).as_deref(), Ok("plain"));
    assert_eq!(substitute("", named).as_deref(), Ok(""));
    assert_eq!(
        substitute("a={revision}/{platform}", named).as_deref(),
        Ok("a=<revision>/<platform>")
    );
    assert_eq!(
        substitute("{{x}} {{{platform}}} }}{{", named).as_deref(),
        Ok("{x} {<platform>} }{")
    );
    // Multi-byte text around the braces is kept whole.
    assert_eq!(
        substitute("\u{e9}{{\u{e9}}}{revision}\u{e9}", named).as_deref(),
        Ok("\u{e9}{\u{e9}}<revision>\u{e9}")
    );
}

#[test]
fn substitution_refuses_what_the_scanner_refuses_with_the_same_error() {
    for text in [
        "{nope}",
        "{}",
        "a}",
        "{revision",
        "{revision{platform}}",
        "}",
    ] {
        assert_eq!(
            substitute(text, named).map(drop),
            placeholders(text).map(drop),
            "{text}"
        );
        assert!(substitute(text, named).is_err(), "{text}");
    }
}

#[test]
fn substitution_asks_for_the_placeholders_in_the_order_they_occur() {
    let asked = std::cell::RefCell::new(Vec::new());
    let out = substitute("{platform}{revision}{platform}", |p| {
        asked.borrow_mut().push(p);
        String::new()
    });
    assert_eq!(out.as_deref(), Ok(""));
    assert_eq!(
        asked.into_inner(),
        [
            Placeholder::Platform,
            Placeholder::Revision,
            Placeholder::Platform
        ]
    );
}
