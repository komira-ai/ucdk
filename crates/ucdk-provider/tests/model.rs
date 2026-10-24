// SPDX-License-Identifier: Apache-2.0
//! The node model: paths, keys, digests, stamps and errors.

use ucdk_provider::{
    DesiredField, FieldValue, Kind, ModelError, Node, NodeKey, NodePath, OwnerLabel, OwnerScope,
    PhysicalId, ProviderError, Refusal, Stamp, StampEvidence,
};

#[test]
fn a_path_is_one_or_more_valid_segments() {
    for ok in [
        "a",
        "app/web",
        "a-1/b2/c-d",
        &"a".repeat(32),
        "a/b/c/d/e/f/g/h/i/j/k/l/m",
    ] {
        assert_eq!(NodePath::new(ok).unwrap().as_str(), ok);
    }
    assert_eq!(NodePath::new(""), Err(ModelError::EmptyPath));
    for bad in ["A", "1a", "a b", "a//b", "a/", "/a", "a_b", &"a".repeat(33)] {
        assert!(
            matches!(NodePath::new(bad), Err(ModelError::BadSegment(_))),
            "{bad}"
        );
    }
    assert_eq!(
        NodePath::new("app/Web"),
        Err(ModelError::BadSegment("Web".to_owned()))
    );
}

#[test]
fn the_owner_is_the_first_segment() {
    let path = NodePath::new("app/web/role").unwrap();
    assert_eq!(path.owner().as_str(), "app");
    assert_eq!(path.owner(), OwnerLabel::new("app").unwrap());
    assert!(OwnerLabel::new("App").is_err());
    assert_eq!(path.to_string(), "app/web/role");
    assert_eq!(path.owner().to_string(), "app");
}

#[test]
fn a_node_key_is_26_base32_characters_that_depend_on_the_whole_path() {
    let key = NodePath::new("app/web").unwrap().key();
    assert_eq!(key.as_str().len(), 26);
    assert!(
        key.as_str()
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
    );
    assert_eq!(key, NodeKey::of(&NodePath::new("app/web").unwrap()));
    for other in ["app/web2", "app/we", "web/app", "app"] {
        assert_ne!(key, NodePath::new(other).unwrap().key(), "{other}");
    }
    assert_eq!(key.to_string(), key.as_str());
}

#[test]
fn a_node_key_is_pinned_to_its_scheme() {
    // The scheme is a promise to every object already stamped: this value must not change.
    assert_eq!(
        NodePath::new("app/web").unwrap().key().as_str(),
        "5sxo3w6l42g34qiap3fg3dioqu"
    );
}

#[test]
fn a_key_is_read_back_only_in_its_own_shape() {
    let key = NodePath::new("app/web").unwrap().key();
    assert_eq!(NodeKey::from_text(key.as_str()), Some(key));
    for bad in [
        "",
        "short",
        &"a".repeat(27),
        &"A".repeat(26),
        &"1".repeat(26),
    ] {
        assert_eq!(NodeKey::from_text(bad), None, "{bad}");
    }
}

#[test]
fn identifiers_print_as_they_were_given() {
    assert_eq!(PhysicalId::new("p-1").to_string(), "p-1");
    assert_eq!(PhysicalId::new("p-1").as_str(), "p-1");
    let node = Node::new(NodePath::new("a/b").unwrap(), Kind::service(), "n", vec![]).unwrap();
    assert_eq!(node.digest().to_string(), node.digest().as_str());
    assert_eq!(node.digest().as_str().len(), 64);
    assert_eq!(Kind::new("x").as_str(), "x");
}

#[test]
fn a_node_keeps_its_fields_sorted_and_refuses_a_name_twice() {
    let fields = vec![
        DesiredField::new("b", FieldValue::Int(1)),
        DesiredField::new("a", FieldValue::Bool(true)),
    ];
    let node = Node::new(NodePath::new("a/b").unwrap(), Kind::job(), "n", fields).unwrap();
    let names: Vec<&str> = node.fields().iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(
        (node.name(), node.kind(), node.path().as_str()),
        ("n", &Kind::job(), "a/b")
    );

    let twice = vec![
        DesiredField::new("a", FieldValue::Int(1)),
        DesiredField::new("a", FieldValue::Int(2)),
    ];
    assert_eq!(
        Node::new(NodePath::new("a/b").unwrap(), Kind::job(), "n", twice),
        Err(ModelError::DuplicateField("a".to_owned()))
    );
}

#[test]
fn field_order_given_does_not_change_the_digest() {
    let one = DesiredField::new("a", FieldValue::Int(1));
    let two = DesiredField::new("b", FieldValue::Int(2));
    let path = || NodePath::new("a/b").unwrap();
    let x = Node::new(path(), Kind::job(), "n", vec![one.clone(), two.clone()]).unwrap();
    let y = Node::new(path(), Kind::job(), "n", vec![two, one]).unwrap();
    assert_eq!(x.digest(), y.digest());
}

#[test]
fn field_boundaries_are_part_of_the_digest() {
    let path = || NodePath::new("a/b").unwrap();
    let digest = |name: &str, text: &str| {
        let f = vec![DesiredField::new(name, FieldValue::Text(text.to_owned()))];
        Node::new(path(), Kind::job(), "n", f).unwrap().digest()
    };
    assert_ne!(digest("ab", "c"), digest("a", "bc"));
}

#[test]
fn a_stamp_names_owner_node_and_scheme_and_ignores_extra_labels_when_compared() {
    let path = NodePath::new("app/web").unwrap();
    let plain = Stamp::for_node(&path);
    assert_eq!(plain.scheme, 1);
    assert_eq!(plain.owner.as_str(), "app");
    assert_eq!(plain.node, path.key());
    let annotated = Stamp::for_node(&path).with_extra("run", "r1");
    assert!(plain.same_identity(&annotated));
    assert_ne!(plain, annotated);
    assert!(!plain.same_identity(&Stamp::for_node(&NodePath::new("app/api").unwrap())));
    assert!(!plain.same_identity(&Stamp::for_node(&NodePath::new("lib/web").unwrap())));
    let other_scheme = Stamp {
        scheme: 2,
        ..plain.clone()
    };
    assert!(!plain.same_identity(&other_scheme));
}

#[test]
fn a_stamp_survives_being_encoded_in_a_name() {
    let stamp = Stamp::for_node(&NodePath::new("app/web").unwrap());
    let name = stamp.encode_name("my~job");
    assert_eq!(
        name, "my~job~ucdk1~app~5sxo3w6l42g34qiap3fg3dioqu",
        "a base name may itself hold the separator"
    );
    assert_eq!(
        Stamp::decode_name(&name),
        ("my~job".to_owned(), Some(stamp))
    );
}

#[test]
fn a_name_without_a_whole_stamp_decodes_to_itself() {
    let key = NodePath::new("app/web").unwrap().key();
    for name in [
        "plain".to_owned(),
        "a~b".to_owned(),
        format!("x~ucdk1~app~{}", "short"),
        format!("x~ucdk1~App~{key}"),
        format!("x~ucdk1~app~{}", key.as_str().to_uppercase()),
        format!("x~ucdk~app~{key}"),
        format!("x~ucdkone~app~{key}"),
        format!("x~app~{key}"),
        format!("app~{key}"),
    ] {
        assert_eq!(Stamp::decode_name(&name), (name.clone(), None), "{name}");
    }
}

#[test]
fn errors_say_what_they_are_and_which_refusal() {
    assert_eq!(
        ProviderError::Refused(Refusal::NotFound).refusal(),
        Some(Refusal::NotFound)
    );
    for e in [
        ProviderError::Permanent("p".into()),
        ProviderError::Transient("t".into()),
        ProviderError::InFlight("i".into()),
        ProviderError::Unknown("u".into()),
    ] {
        assert_eq!(e.refusal(), None);
    }
    let shown: Vec<String> = [
        ProviderError::Refused(Refusal::AlreadyExists),
        ProviderError::Permanent("p".into()),
        ProviderError::Transient("t".into()),
        ProviderError::InFlight("i".into()),
        ProviderError::Unknown("u".into()),
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        shown,
        [
            "refused: AlreadyExists",
            "permanent: p",
            "transient: t",
            "in flight: i",
            "unknown: u"
        ]
    );
    let model: Vec<String> = [
        ModelError::EmptyPath,
        ModelError::BadSegment("X".into()),
        ModelError::DuplicateField("f".into()),
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        model,
        [
            "a node path is empty",
            "`X` is not a valid path segment",
            "field `f` is given twice"
        ]
    );
}

#[test]
fn an_owner_scope_names_one_owner() {
    let owner = OwnerLabel::new("app").unwrap();
    assert_eq!(OwnerScope::new(owner.clone()).owner, owner);
    assert_eq!(StampEvidence::Absent, StampEvidence::Absent);
}
