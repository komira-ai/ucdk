// SPDX-License-Identifier: Apache-2.0
//! The echo guard: when a registry's text may be quoted in a diagnostic, and when it is withheld
//! because it holds the credential of the request. Every credential here is an obvious placeholder.

use ucdk_release_mopm::{
    DETAIL_EXCERPT_BYTES, ECHO_WINDOW_BYTES, Secret, WITHHELD_ECHO, echoes_credential,
    excerpt_unless_echoes, withhold_if_echoes,
};

const TOKEN: &[u8] = b"PLACEHOLDER-TOKEN-0123456789-abcdef";

fn auth() -> Secret {
    Secret::bearer(TOKEN).unwrap()
}

fn body_with(text: &str) -> Vec<u8> {
    format!(r#"{{"error":{{"code":"invalid_request","message":"{text}"}}}}"#).into_bytes()
}

fn token() -> &'static str {
    std::str::from_utf8(TOKEN).unwrap()
}

#[test]
fn the_window_is_sixteen_bytes() {
    assert_eq!(ECHO_WINDOW_BYTES, 16);
}

#[test]
fn a_body_that_echoes_the_token_is_withheld() {
    let body = body_with(&format!("bad credential {}", token()));
    assert!(echoes_credential(&body, &auth()));
    assert_eq!(excerpt_unless_echoes(&body, &auth()), WITHHELD_ECHO);
}

#[test]
fn a_body_that_echoes_the_whole_authorization_value_is_withheld() {
    let body = body_with(&format!("bad header Bearer {}", token()));
    assert_eq!(excerpt_unless_echoes(&body, &auth()), WITHHELD_ECHO);
}

#[test]
fn an_echo_the_server_cut_short_is_withheld() {
    let cut = &token()[..20];
    let body = body_with(&format!("bad credential {cut}..."));
    assert_eq!(excerpt_unless_echoes(&body, &auth()), WITHHELD_ECHO);
}

#[test]
fn an_echo_from_the_middle_of_the_token_is_withheld() {
    let middle = &token()[10..30];
    let body = body_with(&format!("...{middle}..."));
    assert_eq!(excerpt_unless_echoes(&body, &auth()), WITHHELD_ECHO);
}

#[test]
fn an_echo_past_the_end_of_the_excerpt_is_withheld() {
    let mut body = vec![b'x'; DETAIL_EXCERPT_BYTES + 100];
    body.extend_from_slice(TOKEN);
    assert_eq!(excerpt_unless_echoes(&body, &auth()), WITHHELD_ECHO);
}

#[test]
fn a_run_of_fifteen_bytes_is_quoted_and_a_run_of_sixteen_is_withheld() {
    let fifteen = &token()[..ECHO_WINDOW_BYTES - 1];
    let sixteen = &token()[..ECHO_WINDOW_BYTES];
    assert!(!echoes_credential(fifteen.as_bytes(), &auth()));
    assert!(echoes_credential(sixteen.as_bytes(), &auth()));
}

#[test]
fn a_short_credential_is_matched_whole() {
    let auth = Secret::bearer(b"sh0rt").unwrap();
    assert!(echoes_credential(b"the value sh0rt was refused", &auth));
    assert!(!echoes_credential(b"the value sh0r was refused", &auth));
}

#[test]
fn a_credential_with_no_scheme_is_matched_whole_and_by_runs() {
    let auth = Secret::new(TOKEN.to_vec());
    assert!(echoes_credential(&TOKEN[3..30], &auth));
    assert!(!echoes_credential(b"nothing of it here", &auth));
}

#[test]
fn a_part_of_the_credential_shorter_than_four_bytes_is_not_a_shape() {
    let auth = Secret::new(b"Basic ab".to_vec());
    assert!(!echoes_credential(b"it is ab alone", &auth));
    assert!(echoes_credential(b"the header Basic ab was sent", &auth));
}

#[test]
fn a_request_with_no_credential_withholds_nothing() {
    let none = Secret::new(Vec::new());
    assert!(!echoes_credential(b"anything at all", &none));
    let scheme_only = Secret::new(b"Bearer ".to_vec());
    assert!(!echoes_credential(b"Bearer", &scheme_only));
}

#[test]
fn a_body_that_does_not_echo_is_quoted() {
    let body = body_with("no such package");
    assert_eq!(
        excerpt_unless_echoes(&body, &auth()),
        String::from_utf8(body).unwrap()
    );
}

#[test]
fn a_quoted_body_shows_only_printable_ascii_on_one_line() {
    let body = b"line one\r\n\tline\x00two \xe2\x98\x85";
    assert_eq!(
        excerpt_unless_echoes(body, &auth()),
        "line one   line?two ???"
    );
}

#[test]
fn a_quoted_body_is_cut_at_the_bound_and_says_so() {
    let body = vec![b'x'; DETAIL_EXCERPT_BYTES + 1];
    let excerpt = excerpt_unless_echoes(&body, &auth());
    assert_eq!(excerpt, format!("{}...", "x".repeat(DETAIL_EXCERPT_BYTES)));
    let exact = vec![b'x'; DETAIL_EXCERPT_BYTES];
    assert_eq!(
        excerpt_unless_echoes(&exact, &auth()),
        "x".repeat(DETAIL_EXCERPT_BYTES)
    );
}

#[test]
fn a_message_that_echoes_the_token_is_withheld() {
    let message = format!(
        "connection to the registry failed after sending {}",
        token()
    );
    assert_eq!(withhold_if_echoes(message, &auth()), WITHHELD_ECHO);
}

#[test]
fn a_message_that_does_not_echo_is_kept() {
    let message = "connection reset by peer".to_owned();
    assert_eq!(withhold_if_echoes(message.clone(), &auth()), message);
}
