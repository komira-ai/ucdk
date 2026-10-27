// SPDX-License-Identifier: Apache-2.0
//! The transport types: secrets that cannot be printed, and the shape of a request and answer.

mod common;

use common::response;
use ucdk_release_mopm::{
    Header, HeaderValue, Method, Request, Secret, TransportError, TransportKind, Url,
    encode_segment,
};

const TOKEN: &[u8] = b"CANARY-bearer-token-0123";
const SIGNATURE: &str = "CANARY-signature-4567";

#[test]
fn a_bearer_is_built_from_a_visible_token_only() {
    let secret = Secret::bearer(TOKEN).unwrap();
    assert_eq!(secret.expose(), b"Bearer CANARY-bearer-token-0123");
    assert_eq!(Secret::new(b"x".to_vec()).expose(), b"x");
    for bad in [
        &b""[..],
        b"a b",
        b"a\nb",
        b"a\r\nb",
        "t\u{e9}".as_bytes(),
        b"a\0b",
        b"\x7f",
    ] {
        assert!(Secret::bearer(bad).is_none(), "{bad:?}");
        assert!(Header::bearer(bad).is_none(), "{bad:?}");
    }
}

#[test]
fn a_bearer_header_is_an_authorization_secret() {
    let header = Header::bearer(TOKEN).unwrap();
    assert_eq!(header.name, "authorization");
    assert!(matches!(header.value, HeaderValue::Secret(_)));
    let plain = Header::plain("accept", "application/json");
    assert!(matches!(&plain.value, HeaderValue::Plain(v) if v == "application/json"));
}

#[test]
fn a_request_never_prints_its_token_or_its_signature() {
    let url = Url::parse(&format!(
        "https://files.example.com/store/a.conda?X-Amz-Signature={SIGNATURE}&X-Amz-Expires=300"
    ))
    .unwrap();
    let body = b"file bytes";
    let request = Request {
        method: Method::Put,
        url,
        headers: vec![
            Header::plain("content-length", "10"),
            Header::bearer(TOKEN).unwrap(),
        ],
        body,
    };
    let shown = format!("{request:?}");
    assert!(!shown.contains("CANARY"), "{shown}");
    assert!(shown.contains("PUT"), "{shown}");
    assert!(
        shown.contains("https://files.example.com/store/a.conda?<redacted>"),
        "{shown}"
    );
    assert!(shown.contains("content-length: 10"), "{shown}");
    assert!(shown.contains("authorization: <redacted>"), "{shown}");
    assert!(shown.contains("body_len: 10"), "{shown}");
    // The wire form still has the query, for the adapter that sends it.
    assert!(request.url.expose().contains(SIGNATURE));
    assert!(!request.url.redacted().contains(SIGNATURE));
}

#[test]
fn a_url_without_a_query_prints_as_it_is() {
    let url = Url::parse("https://registry.example.com:8443/api/v1/packages").unwrap();
    assert_eq!(
        url.redacted(),
        "https://registry.example.com:8443/api/v1/packages"
    );
    assert_eq!(url.expose(), url.redacted());
    let joined = url.join(&["demo", "1.2.3"]);
    assert_eq!(
        joined.expose(),
        "https://registry.example.com:8443/api/v1/packages/demo/1.2.3"
    );
    let with_query = Url::parse("https://registry.example.com/x?a=1").unwrap();
    assert!(with_query.has_query());
    assert_eq!(
        with_query.join(&["y"]).expose(),
        "https://registry.example.com/x/y"
    );
}

#[test]
fn a_segment_is_percent_encoded_byte_by_byte() {
    assert_eq!(encode_segment("demo_1.2.3-x~"), "demo_1.2.3-x~");
    assert_eq!(encode_segment("a/b c"), "a%2Fb%20c");
    assert_eq!(encode_segment("\u{e9}"), "%C3%A9");
    assert_eq!(encode_segment(""), "");
}

#[test]
fn methods_are_named_as_on_the_wire() {
    let names: Vec<_> = [Method::Get, Method::Post, Method::Put, Method::Delete]
        .into_iter()
        .map(Method::as_str)
        .collect();
    assert_eq!(names, ["GET", "POST", "PUT", "DELETE"]);
}

#[test]
fn a_response_finds_a_header_without_regard_to_case() {
    let answer = response(200, &[("Content-Type", "x"), ("Retry-After", "5")], "");
    assert_eq!(answer.header("retry-after"), Some("5"));
    assert_eq!(answer.header("RETRY-AFTER"), Some("5"));
    assert_eq!(answer.header("content-type"), Some("x"));
    assert_eq!(answer.header("etag"), None);
}

#[test]
fn only_2xx_is_success() {
    for (status, ok) in [
        (199, false),
        (200, true),
        (204, true),
        (299, true),
        (300, false),
        (404, false),
    ] {
        assert_eq!(response(status, &[], "").is_success(), ok, "{status}");
    }
}

#[test]
fn a_transport_error_says_how_far_the_request_got() {
    let never = TransportError::never_connected("refused");
    assert_eq!(never.kind(), TransportKind::NeverConnected);
    assert_eq!(never.message(), "refused");
    assert_eq!(never.to_string(), "no connection: refused");
    let lost = TransportError::no_usable_answer(String::from("timed out"));
    assert_eq!(lost.kind(), TransportKind::NoUsableAnswer);
    assert_eq!(lost.to_string(), "no usable answer: timed out");
}
