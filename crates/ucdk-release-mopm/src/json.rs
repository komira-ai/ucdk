// SPDX-License-Identifier: Apache-2.0
//! A JSON object read for a credential: a name that appears twice in any object, at any depth,
//! is noted, so that two readers can never disagree about which value counts, and the one field that holds a credential is
//! read into a buffer that is wiped on drop, whether the rest of the object is read or not.
//!
//! Not held: a credential written with a JSON escape passes through `serde_json`'s scratch
//! buffer, which this crate cannot wipe. A credential with no escape is copied once, from the
//! body (which the caller wipes) straight into the wiped buffer.

use std::cell::Cell;
use std::fmt;

use serde::de::{DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use zeroize::Zeroizing;

/// The fields of one JSON object.
pub(crate) struct Fields {
    /// The credential field, when it is present and is text.
    pub secret: Option<Zeroizing<String>>,
    /// Every other field.
    pub rest: Map<String, Value>,
    /// A name appeared more than once in one object, at any depth.
    pub repeated: bool,
}

/// Reads the credential field: text into a wiped buffer, any other value into nothing (a value
/// that is not text holds no credential, and it is skipped without being kept).
struct Credential;

impl<'de> DeserializeSeed<'de> for Credential {
    type Value = Option<Zeroizing<String>>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Credential {
    type Value = Option<Zeroizing<String>>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
        let mut wiped = Zeroizing::new(String::with_capacity(text.len()));
        wiped.push_str(text);
        Ok(Some(wiped))
    }

    fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(None)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(None)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(None)
    }
}

/// Reads any value into a `Value`, as `serde_json` would, but notes in `repeated` a name that
/// appears twice in one of its objects, at any depth (where a plain `Value` keeps the last).
struct Strict<'a> {
    repeated: &'a Cell<bool>,
}

impl<'de> DeserializeSeed<'de> for Strict<'_> {
    type Value = Value;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Strict<'_> {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::from(v))
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element_seed(Strict {
            repeated: self.repeated,
        })? {
            items.push(item);
        }
        Ok(Value::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
        let mut map = Map::new();
        while let Some(name) = access.next_key::<String>()? {
            let value = access.next_value_seed(Strict {
                repeated: self.repeated,
            })?;
            if map.insert(name, value).is_some() {
                self.repeated.set(true);
            }
        }
        Ok(Value::Object(map))
    }
}

/// Reads one object; the field named `secret`, if any, is the credential.
struct Object<'a> {
    secret: Option<&'a str>,
}

impl<'de> Visitor<'de> for Object<'_> {
    type Value = Fields;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Fields, A::Error> {
        let mut fields = Fields {
            secret: None,
            rest: Map::new(),
            repeated: false,
        };
        let mut seen_secret = false;
        let nested = Cell::new(false);
        while let Some(name) = access.next_key::<String>()? {
            if Some(name.as_str()) == self.secret {
                // The earlier value, if any, is dropped here and so wiped.
                fields.secret = access.next_value_seed(Credential)?;
                fields.repeated |= seen_secret;
                seen_secret = true;
            } else {
                let value = access.next_value_seed(Strict { repeated: &nested })?;
                fields.repeated |= fields.rest.insert(name, value).is_some();
            }
        }
        fields.repeated |= nested.get();
        Ok(fields)
    }
}

/// The fields of `body`, which must be exactly one JSON object; `None` for anything else. The
/// field named `secret`, if any, is read into a wiped buffer.
pub(crate) fn read_object(body: &[u8], secret: Option<&str>) -> Option<Fields> {
    let mut de = serde_json::Deserializer::from_slice(body);
    let fields = de.deserialize_map(Object { secret }).ok()?;
    de.end().ok()?;
    Some(fields)
}

#[cfg(test)]
mod tests {
    use serde::de::Expected;

    use super::*;

    fn read(body: &str, secret: Option<&str>) -> Option<Fields> {
        read_object(body.as_bytes(), secret)
    }

    #[test]
    fn the_credential_is_read_into_wiped_storage_and_the_rest_into_values() {
        let fields = read(r#"{"token":"t\/1","n":1,"o":{"a":[1]}}"#, Some("token")).unwrap();
        // This line compiles only while the credential is held in `Zeroizing`.
        let secret: &Zeroizing<String> = fields.secret.as_ref().unwrap();
        assert_eq!(secret.as_str(), "t/1");
        assert_eq!(fields.rest.len(), 2);
        assert!(!fields.repeated);
        let plain = read(r#"{"token":"abc"}"#, Some("token")).unwrap();
        assert_eq!(plain.secret.as_ref().map(|s| s.as_str()), Some("abc"));
        assert!(plain.rest.is_empty());
    }

    #[test]
    fn a_credential_field_that_is_not_text_is_no_credential() {
        for value in [
            "5",
            "-5",
            "1.5",
            "true",
            "null",
            "[1,[2]]",
            r#"{"a":{"b":1}}"#,
        ] {
            let body = format!(r#"{{"token":{value},"after":1}}"#);
            let fields = read(&body, Some("token")).unwrap();
            assert!(fields.secret.is_none(), "{value}");
            assert_eq!(fields.rest.len(), 1, "{value}");
        }
    }

    #[test]
    fn a_repeated_name_is_noted_for_the_credential_and_for_any_other_field() {
        let twice = read(r#"{"token":"a","token":"b"}"#, Some("token")).unwrap();
        assert!(twice.repeated);
        assert!(
            read(r#"{"token":5,"token":"b"}"#, Some("token"))
                .unwrap()
                .repeated
        );
        assert!(read(r#"{"a":1,"a":1}"#, Some("token")).unwrap().repeated);
        assert!(read(r#"{"a":1,"a":1}"#, None).unwrap().repeated);
        assert!(!read(r#"{"a":1,"b":1}"#, None).unwrap().repeated);
    }

    #[test]
    fn a_repeated_name_at_any_depth_is_noted_and_the_values_are_kept_as_written() {
        for body in [
            r#"{"p":[{"name":"other","name":"alpha","version":"1"}]}"#,
            r#"{"p":[{"name":"alpha","version":"1","name":"other"}]}"#,
            r#"{"p":[{"name":"alpha","version":"1","version":"2"}]}"#,
            r#"{"p":{"q":{"a":1,"a":2}}}"#,
            r#"{"p":[[{"a":1,"a":1}]]}"#,
        ] {
            assert!(read(body, Some("token")).unwrap().repeated, "{body}");
        }
        // Equal names in different objects, and every kind of value, are no repeat.
        let body = r#"{"p":[{"name":"a","x":null},{"name":"b","x":[true,-1,2,1.5,"t",{}]}]}"#;
        let fields = read(body, Some("token")).unwrap();
        assert!(!fields.repeated);
        let expected: Value = serde_json::from_str(body).unwrap();
        assert_eq!(fields.rest["p"], expected["p"]);
    }

    #[test]
    fn anything_but_one_object_is_not_read() {
        for body in [
            "",
            "[]",
            "\"t\"",
            "{",
            r#"{"a":1} x"#,
            r#"{"token":"a""#,
            r#"{"token":"a",}"#,
            r#"{"token":[1,}"#,
            r#"{"token":{"a":}}"#,
        ] {
            assert!(read(body, Some("token")).is_none(), "{body}");
        }
    }

    #[test]
    fn each_reader_says_what_it_expects() {
        let object: &dyn Expected = &Object { secret: None };
        assert_eq!(object.to_string(), "a JSON object");
        let credential: &dyn Expected = &Credential;
        assert_eq!(credential.to_string(), "any JSON value");
        let repeated = Cell::new(false);
        let strict: &dyn Expected = &Strict {
            repeated: &repeated,
        };
        assert_eq!(strict.to_string(), "any JSON value");
    }
}
