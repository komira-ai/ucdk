// SPDX-License-Identifier: Apache-2.0
//! The node model: paths, keys, digests, stamps and errors.

use ucdk_provider::kit;
use ucdk_provider::{
    CellName, CellScope, CellUid, DesiredField, FieldValue, Kind, ModelError, NAME_SUFFIX_MAX,
    NODE_KEY_LEN, Node, NodeKey, NodePath, OwnerLabel, OwnerScope, PhysicalId, ProviderError,
    Refusal, SCHEME, SEGMENT_MAX, Stamp, StampEvidence, StampMode,
};

/// The id of `kit::cell()`.
const CELL_UID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaa";

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
fn a_stamp_names_cell_owner_node_and_scheme_and_ignores_extra_labels_when_compared() {
    let path = NodePath::new("app/web").unwrap();
    let cell = kit::cell();
    let plain = Stamp::for_node(&cell, &path);
    assert_eq!(plain.scheme, 1);
    assert_eq!(plain.cell, cell);
    assert_eq!(plain.owner.as_str(), "app");
    assert_eq!(plain.node, path.key());
    let annotated = Stamp::for_node(&cell, &path).with_extra("run", "r1");
    assert!(plain.same_identity(&annotated));
    assert_ne!(plain, annotated);
    let of = |p: &str| Stamp::for_node(&cell, &NodePath::new(p).unwrap());
    assert!(!plain.same_identity(&of("app/api")));
    assert!(!plain.same_identity(&of("lib/web")));
    assert!(!plain.same_identity(&Stamp::for_node(&kit::other_cell(), &path)));
    let other_scheme = Stamp {
        scheme: 2,
        ..plain.clone()
    };
    assert!(!plain.same_identity(&other_scheme));
}

#[test]
fn a_stamp_survives_being_encoded_in_a_name() {
    let stamp = Stamp::for_node(&kit::cell(), &NodePath::new("app/web").unwrap());
    let name = stamp.encode_name("my~job");
    assert_eq!(
        name,
        format!("my~job~ucdk1~kit-machine~kit-cell~{CELL_UID}~app~5sxo3w6l42g34qiap3fg3dioqu"),
        "a base name may itself hold the separator"
    );
    assert_eq!(
        Stamp::decode_name(&name),
        ("my~job".to_owned(), Some(stamp))
    );
}

#[test]
fn the_name_suffix_budget_covers_the_longest_stamp() {
    // The longest stamp: every segment at its limit.
    let long = "a".repeat(SEGMENT_MAX);
    let cell = ucdk_provider::CellIdentity::new(
        CellName::new(&long).unwrap(),
        CellName::new(&long).unwrap(),
        CellUid::from_text(CELL_UID).unwrap(),
    );
    let path = NodePath::new(&format!("{long}/{long}")).unwrap();
    let stamp = Stamp::for_node(&cell, &path);
    let name = stamp.encode_name("base");
    assert_eq!(NAME_SUFFIX_MAX, 159);
    assert_eq!(stamp.name_suffix_len(), NAME_SUFFIX_MAX);
    assert_eq!(name.len(), "base".len() + NAME_SUFFIX_MAX);
    assert_eq!(Stamp::decode_name(&name), ("base".to_owned(), Some(stamp)));

    // A short stamp adds less, never more.
    let short = Stamp::for_node(&kit::cell(), &NodePath::new("a/b").unwrap());
    assert!(short.name_suffix_len() < NAME_SUFFIX_MAX);
}

#[test]
fn a_name_without_a_whole_stamp_decodes_to_itself() {
    let key = NodePath::new("app/web").unwrap().key();
    let cell = format!("kit-machine~kit-cell~{CELL_UID}");
    for name in [
        "plain".to_owned(),
        "a~b".to_owned(),
        format!("x~ucdk1~{cell}~app~{}", "short"),
        format!("x~ucdk1~{cell}~App~{key}"),
        format!("x~ucdk1~{cell}~app~{}", key.as_str().to_uppercase()),
        format!("x~ucdk~{cell}~app~{key}"),
        format!("x~ucdkone~{cell}~app~{key}"),
        format!("x~{cell}~app~{key}"),
        format!("x~ucdk1~Kit-machine~kit-cell~{CELL_UID}~app~{key}"),
        format!("x~ucdk1~kit-machine~Kit-cell~{CELL_UID}~app~{key}"),
        format!("x~ucdk1~kit-machine~kit-cell~short~app~{key}"),
        format!("x~ucdk1~kit-cell~{CELL_UID}~app~{key}"),
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
        ModelError::BadCellUid("u".into()),
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(
        model,
        [
            "a node path is empty",
            "`X` is not a valid path segment",
            "field `f` is given twice",
            "`u` is not a valid cell id"
        ]
    );
}

#[test]
fn an_owner_scope_names_one_owner_in_one_cell() {
    let owner = OwnerLabel::new("app").unwrap();
    let scope = OwnerScope::new(kit::cell(), owner.clone());
    assert_eq!((scope.owner, scope.cell), (owner, kit::cell()));
    assert_eq!(StampEvidence::Absent, StampEvidence::Absent);
}

#[test]
fn a_cell_scope_names_one_cell() {
    assert_eq!(CellScope::new(kit::cell()).cell, kit::cell());
    assert_ne!(
        CellScope::new(kit::cell()),
        CellScope::new(kit::other_cell())
    );
}

#[test]
fn only_a_tag_carries_extra_labels() {
    assert!(StampMode::Tag.carries_extra());
    assert!(!StampMode::NameEncoded.carries_extra());
    assert!(!StampMode::Embedded.carries_extra());
}

#[test]
fn only_stamped_modes_are_applicable_today() {
    assert!(StampMode::Tag.applicable());
    assert!(StampMode::NameEncoded.applicable());
    assert!(!StampMode::Embedded.applicable());
}

#[test]
fn a_cell_is_a_machine_a_name_and_a_26_character_id() {
    let cell = kit::cell();
    assert_eq!(cell.machine(), &CellName::new("kit-machine").unwrap());
    assert_eq!(cell.cell().as_str(), "kit-cell");
    assert_eq!(cell.uid().as_str(), CELL_UID);
    assert_eq!(cell.uid().to_string(), CELL_UID);
    assert_eq!(cell.to_string(), format!("kit-machine/kit-cell/{CELL_UID}"));
    assert_ne!(cell, kit::other_cell());
    assert_eq!(
        CellName::new("Cell"),
        Err(ModelError::BadSegment("Cell".to_owned()))
    );
    assert!(CellName::new(&"a".repeat(33)).is_err());
    for bad in [
        "",
        "short",
        &"a".repeat(27),
        &"A".repeat(26),
        &"1".repeat(26),
    ] {
        assert_eq!(
            CellUid::from_text(bad),
            Err(ModelError::BadCellUid(bad.to_owned())),
            "{bad}"
        );
    }
}

#[test]
fn the_format_constants_are_pinned() {
    // These values reach cloud tags, physical names and the ledger: changing one is a format
    // break for every object already stamped.
    assert_eq!(SEGMENT_MAX, 32);
    assert_eq!(NODE_KEY_LEN, 26);
    assert_eq!(SCHEME, 1);
    assert_eq!(
        Stamp::for_node(&kit::cell(), &NodePath::new("a/b").unwrap()).scheme,
        SCHEME
    );
    assert!(NodePath::new(&"a".repeat(SEGMENT_MAX)).is_ok());
    assert!(NodePath::new(&"a".repeat(SEGMENT_MAX + 1)).is_err());
    assert!(CellName::new(&"a".repeat(SEGMENT_MAX)).is_ok());
    assert!(CellName::new(&"a".repeat(SEGMENT_MAX + 1)).is_err());
    assert!(OwnerLabel::new(&"a".repeat(SEGMENT_MAX)).is_ok());
    assert!(OwnerLabel::new(&"a".repeat(SEGMENT_MAX + 1)).is_err());
    let path = NodePath::new("app/web").unwrap();
    assert_eq!(path.key().as_str().len(), NODE_KEY_LEN);
}
