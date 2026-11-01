// SPDX-License-Identifier: Apache-2.0
//! What `redact` replaces in the output of a tool, and what it leaves alone. Every secret here is
//! an obvious placeholder.

use ucdk_release_build::{REDACTED, redact};

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
            redact(&format!("https://h.example/p?{key}=PLACEHOLDER-1&n=1")),
            format!("https://h.example/p?{key}={REDACTED}&n=1"),
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
    same("token= and password=&x=1");
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
    let body = r#"{"error":{"code":"invalid_request","message":"bad url https://h.example/p?access_token=PLACEHOLDER-1 for Bearer PLACEHOLDER-2"}}"#;
    let out = redact(body);
    assert!(!out.contains("PLACEHOLDER"), "{out}");
    assert!(out.contains(r#""code":"invalid_request""#), "{out}");
}

#[test]
fn the_userinfo_of_a_url_is_replaced() {
    assert_eq!(
        redact("fetch https://user:PLACEHOLDER-1@host.example/p failed"),
        "fetch https://[REDACTED]@host.example/p failed"
    );
    assert_eq!(
        redact("git clone ssh://PLACEHOLDER-1@host.example:22/r"),
        "git clone ssh://[REDACTED]@host.example:22/r"
    );
}

#[test]
fn an_at_sign_outside_the_authority_of_a_url_is_left_alone() {
    same("https://host.example/a@b");
    same("https://host.example/?e=a@b");
    same("https://host.example#a@b");
    same("mail me@host.example about https://host.example");
}

#[test]
fn every_url_of_a_text_is_checked() {
    assert_eq!(
        redact(
            "https://a:PLACEHOLDER-1@x.example https://y.example https://b:PLACEHOLDER-2@z.example"
        ),
        "https://[REDACTED]@x.example https://y.example https://[REDACTED]@z.example"
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
    let once =
        redact("Bearer PLACEHOLDER-1 https://u:PLACEHOLDER-2@h.example/?token=PLACEHOLDER-3");
    assert_eq!(redact(&once), once);
}
