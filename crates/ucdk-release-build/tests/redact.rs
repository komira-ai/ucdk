// SPDX-License-Identifier: Apache-2.0
//! What `redact` replaces in the output of a tool, and what it leaves alone. Every secret here is
//! an obvious placeholder.

use ucdk_release_build::{REDACTED, redact};

/// A URL built at run time, so that a sample with userinfo is not a literal URL in the source.
fn url(scheme: &str, rest: &str) -> String {
    format!("{scheme}://{rest}")
}

fn same(text: &str) {
    assert_eq!(redact(text), text);
}

#[test]
fn a_bearer_credential_is_replaced_up_to_the_next_space() {
    assert_eq!(
        redact("Authorization: Bearer PLACEHOLDER-1 was rejected"),
        "Authorization: Bearer [REDACTED] was rejected"
    );
}

#[test]
fn the_bearer_marker_matches_in_any_letter_case() {
    assert_eq!(redact("BEARER PLACEHOLDER-1"), "BEARER [REDACTED]");
    assert_eq!(redact("bearer PLACEHOLDER-1"), "bearer [REDACTED]");
}

#[test]
fn a_bearer_credential_in_quotes_stops_at_the_quote() {
    assert_eq!(
        redact(r#"{"h":"Bearer PLACEHOLDER-1"}"#),
        r#"{"h":"Bearer [REDACTED]"}"#
    );
}

#[test]
fn a_secret_in_a_query_string_is_replaced_up_to_the_next_parameter() {
    assert_eq!(
        redact("GET /v1/x?token=PLACEHOLDER-1&page=2 failed"),
        "GET /v1/x?token=[REDACTED]&page=2 failed"
    );
}

#[test]
fn every_secret_parameter_name_is_replaced() {
    for key in [
        "token",
        "access_token",
        "refresh_token",
        "password",
        "passwd",
        "secret",
        "client_secret",
        "api_key",
        "apikey",
        "signature",
        "X-Amz-Signature",
        "X-Amz-Credential",
    ] {
        assert_eq!(
            redact(&format!("https://h.example.com/p?{key}=PLACEHOLDER-1&n=1")),
            format!("https://h.example.com/p?{key}={REDACTED}&n=1"),
            "{key}"
        );
    }
}

#[test]
fn a_parameter_name_matches_in_any_letter_case() {
    assert_eq!(redact("TOKEN=PLACEHOLDER-1"), "TOKEN=[REDACTED]");
}

#[test]
fn an_assignment_run_ends_at_a_quote_or_a_line_end() {
    assert_eq!(redact("token=PLACEHOLDER-1'x'"), "token=[REDACTED]'x'");
    assert_eq!(redact("token=PLACEHOLDER-1\"x\""), "token=[REDACTED]\"x\"");
    assert_eq!(
        redact("token=PLACEHOLDER-1\nnext line"),
        "token=[REDACTED]\nnext line"
    );
}

#[test]
fn an_assignment_with_no_value_is_left_alone() {
    same("token=&x=1 password=\nnext line token=");
}

#[test]
fn a_secret_member_of_a_json_object_is_replaced() {
    assert_eq!(
        redact(r#"{"token":"PLACEHOLDER-1","n":1}"#),
        r#"{"token":"[REDACTED]","n":1}"#
    );
    assert_eq!(
        redact(r#"{"client_secret": "PLACEHOLDER-1", "n": 1}"#),
        r#"{"client_secret": "[REDACTED]", "n": 1}"#
    );
}

#[test]
fn a_secret_inside_an_error_body_is_replaced() {
    let body = r#"{"error":{"code":"invalid_request","message":"bad url https://h.example.com/p?access_token=PLACEHOLDER-1 for Bearer PLACEHOLDER-2"}}"#;
    let out = redact(body);
    assert!(!out.contains("PLACEHOLDER"), "{out}");
    assert!(out.contains(r#""code":"invalid_request""#), "{out}");
}

#[test]
fn the_userinfo_of_a_url_is_replaced() {
    assert_eq!(
        redact(&format!(
            "fetch {} failed",
            url("https", "user:PLACEHOLDER-1@host.example.com/p")
        )),
        format!(
            "fetch {} failed",
            url("https", "[REDACTED]@host.example.com/p")
        )
    );
    assert_eq!(
        redact(&url("ssh", "PLACEHOLDER-1@host.example.com:22/r")),
        url("ssh", "[REDACTED]@host.example.com:22/r")
    );
}

#[test]
fn an_at_sign_outside_the_authority_of_a_url_is_left_alone() {
    same("https://host.example.com/a@b");
    same("https://host.example.com/?e=a@b");
    same("https://host.example.com#a@b");
    same("mail me@host.example.com about https://host.example.com");
}

#[test]
fn every_url_of_a_text_is_checked() {
    let first = url("https", "a:PLACEHOLDER-1@x.example.com");
    let plain = url("https", "y.example.com");
    let last = url("https", "b:PLACEHOLDER-2@z.example.com");
    assert_eq!(
        redact(&format!("{first} {plain} {last}")),
        format!(
            "{} {plain} {}",
            url("https", "[REDACTED]@x.example.com"),
            url("https", "[REDACTED]@z.example.com")
        )
    );
}

#[test]
fn every_secret_of_a_text_is_replaced() {
    let out = redact("Bearer PLACEHOLDER-1 then token=PLACEHOLDER-2 then Bearer PLACEHOLDER-3");
    assert_eq!(
        out,
        "Bearer [REDACTED] then token=[REDACTED] then Bearer [REDACTED]"
    );
}

#[test]
fn ordinary_text_is_left_alone() {
    same("");
    same("error: build failed in 3 steps");
    same("a token of thanks; secrets are kept; the password is not shown");
}

#[test]
fn text_that_is_not_ascii_is_kept_whole() {
    same("\u{2605} \u{2014} built \u{26d4} ok");
    assert_eq!(
        redact("\u{2605} token=PLACEHOLDER-\u{2605} \u{2014} done"),
        "\u{2605} token=[REDACTED] \u{2014} done"
    );
}

#[test]
fn redacting_twice_changes_nothing() {
    let text = format!(
        "Bearer PLACEHOLDER-1 {}?token=PLACEHOLDER-3",
        url("https", "u:PLACEHOLDER-2@h.example.com/")
    );
    let once = redact(&text);
    assert_eq!(redact(&once), once);
}

#[test]
fn a_quoted_value_is_replaced_to_its_closing_quote() {
    assert_eq!(
        redact(r#"token="PLACEHOLDER-1" next"#),
        r#"token="[REDACTED]" next"#
    );
    assert_eq!(
        redact("password='PLACEHOLDER-1' next"),
        "password='[REDACTED]' next"
    );
    assert_eq!(
        redact(r#"password="two words" next"#),
        r#"password="[REDACTED]" next"#
    );
}

#[test]
fn an_empty_or_unterminated_quoted_value_is_handled() {
    same(r#"token="" next"#);
    assert_eq!(redact(r#"token="PLACEHOLDER-1"#), r#"token="[REDACTED]"#);
}

#[test]
fn an_escaped_quote_does_not_end_a_quoted_value() {
    assert_eq!(
        redact(r#"{"password":"ab\"cd","n":1}"#),
        r#"{"password":"[REDACTED]","n":1}"#
    );
    assert_eq!(redact(r#"password="ab\"#), r#"password="[REDACTED]"#);
}

#[test]
fn white_space_around_the_separator_is_skipped() {
    assert_eq!(redact("TOKEN = PLACEHOLDER-1 x"), "TOKEN = [REDACTED] x");
    assert_eq!(redact("token: PLACEHOLDER-1 x"), "token: [REDACTED] x");
    assert_eq!(redact("token :\tPLACEHOLDER-1 x"), "token :\t[REDACTED] x");
    assert_eq!(
        redact(r#"{"token" : "PLACEHOLDER-1"}"#),
        r#"{"token" : "[REDACTED]"}"#
    );
}

#[test]
fn an_assignment_with_no_value_on_its_line_is_left_alone() {
    same("token=\nPLACEHOLDER-1");
    same("token=   \nPLACEHOLDER-1");
}

#[test]
fn bearer_may_be_followed_by_any_run_of_spaces_and_tabs() {
    assert_eq!(redact("Bearer  PLACEHOLDER-1 x"), "Bearer  [REDACTED] x");
    assert_eq!(redact("Bearer\tPLACEHOLDER-1 x"), "Bearer\t[REDACTED] x");
    assert_eq!(redact("Bearer \t PLACEHOLDER-1"), "Bearer \t [REDACTED]");
    same("Bearer\nPLACEHOLDER-1");
}

#[test]
fn a_command_line_flag_takes_the_next_word() {
    for key in [
        "token",
        "password",
        "passwd",
        "secret",
        "api_key",
        "api-key",
        "apikey",
        "signature",
        "credential",
        "sig",
        "auth",
        "access-token",
    ] {
        assert_eq!(
            redact(&format!("run --{key} PLACEHOLDER-1 --verbose")),
            format!("run --{key} {REDACTED} --verbose"),
            "{key}"
        );
    }
    assert_eq!(
        redact(r#"run --token "PLACEHOLDER-1" x"#),
        r#"run --token "[REDACTED]" x"#
    );
    assert_eq!(
        redact("run --password=PLACEHOLDER-1 x"),
        "run --password=[REDACTED] x"
    );
}

#[test]
fn a_flag_followed_by_another_flag_or_nothing_is_left_alone() {
    same("run --token --verbose");
    same("run --token");
    same("run --token   ");
}

#[test]
fn a_secret_word_that_is_not_a_flag_or_an_assignment_is_left_alone() {
    same("a token of thanks");
    same("the tokens are kept");
    same("tokenizer");
}

#[test]
fn a_header_that_carries_a_key_is_replaced() {
    assert_eq!(
        redact("x-api-key: PLACEHOLDER-1 x"),
        "x-api-key: [REDACTED] x"
    );
    assert_eq!(redact("X-Api-Key:PLACEHOLDER-1"), "X-Api-Key:[REDACTED]");
}

#[test]
fn an_authorization_header_is_replaced_with_or_without_a_scheme() {
    assert_eq!(
        redact("Authorization: PLACEHOLDER-1 rest"),
        "Authorization: [REDACTED] rest"
    );
    assert_eq!(
        redact("Authorization: Basic PLACEHOLDER-1 rest"),
        "Authorization: Basic [REDACTED] rest"
    );
    assert_eq!(
        redact("authorization: digest PLACEHOLDER-1"),
        "authorization: digest [REDACTED]"
    );
    assert_eq!(
        redact("Authorization: Bearer PLACEHOLDER-1 rest"),
        "Authorization: Bearer [REDACTED] rest"
    );
}

#[test]
fn an_authorization_header_with_only_a_scheme_loses_the_scheme() {
    assert_eq!(
        redact("Authorization: Basic\nnext"),
        "Authorization: [REDACTED]\nnext"
    );
}

#[test]
fn a_percent_encoded_separator_is_replaced() {
    assert_eq!(
        redact("x?token%3DPLACEHOLDER-1&n=1"),
        "x?token%3D[REDACTED]&n=1"
    );
    assert_eq!(
        redact("x?token%3aPLACEHOLDER-1&n=1"),
        "x?token%3a[REDACTED]&n=1"
    );
    same("x?token%3Fn");
}

#[test]
fn sig_and_auth_are_secret_names_only_as_whole_words() {
    assert_eq!(redact("sig=PLACEHOLDER-1"), "sig=[REDACTED]");
    assert_eq!(redact("x?auth=PLACEHOLDER-1&n=1"), "x?auth=[REDACTED]&n=1");
    same("design=keep&author=keep&oauth=keep");
}

#[test]
fn a_json_value_that_is_not_a_string_is_replaced() {
    assert_eq!(
        redact(r#"{"token":123456,"n":1}"#),
        r#"{"token":[REDACTED],"n":1}"#
    );
    assert_eq!(redact(r#"{"token":123456}"#), r#"{"token":[REDACTED]}"#);
}

// Known limits, kept as tests so that a change to them is a decision.

#[test]
fn a_json_string_escaped_inside_another_string_is_not_recognised() {
    same(r#"{\"token\":\"PLACEHOLDER-1\"}"#);
}

#[test]
fn a_userinfo_password_with_a_path_character_is_not_recognised() {
    same(&url("https", "user:PLACEHOLDER/1@host.example.com/p"));
    assert_eq!(
        redact(&url("https", "user:PLACEHOLDER%2F1@host.example.com/p")),
        url("https", "[REDACTED]@host.example.com/p")
    );
}

#[test]
fn prose_after_bearer_or_a_secret_name_and_colon_is_replaced() {
    assert_eq!(
        redact("the bearer of this message"),
        "the bearer [REDACTED] this message"
    );
    assert_eq!(redact("password: required"), "password: [REDACTED]");
}
