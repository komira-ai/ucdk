// SPDX-License-Identifier: Apache-2.0
//! The fake store: a tree under each key, and the paths it refuses.

use ucdk_release::store::{SetScope, StoreKey};
use ucdk_release_fake::FakeStore;
use ucdk_release_proto::v1::Reason;
use ucdk_release_seams::{ArtifactStore, SeamError};

fn scope(generation: &str) -> SetScope {
    SetScope {
        stage: "build".to_owned(),
        platform: "linux-x86_64".to_owned(),
        revision: "abc".to_owned(),
        generation: generation.to_owned(),
    }
}

fn member(generation: &str, name: &str) -> StoreKey {
    StoreKey::member(&scope(generation), name).unwrap()
}

#[test]
fn an_entry_refuses_paths_that_escape_it() {
    let mut store = FakeStore::new();
    let key = member("g1", "tool");
    for bad in [
        "", ".", "..", "../x", "a/../b", "/abs", "a//b", "a\\b", "a\nb",
    ] {
        let result = store.begin(&key).unwrap().put_file(bad, b"x");
        assert_eq!(
            result,
            Err(SeamError::Refused(Reason::SetMember)),
            "{bad:?}"
        );
        store.discard(&key).unwrap();
    }
    assert_eq!(store.files(&key), Ok(Vec::new()));
}

#[test]
fn a_file_and_a_directory_of_one_name_cannot_share_an_entry() {
    let mut store = FakeStore::new();
    let key = member("g1", "tree");
    let refused = Err(SeamError::Refused(Reason::SetMember));
    {
        let mut writer = store.begin(&key).unwrap();
        writer.put_file("a", b"file").unwrap();
        writer.put_file("p/q/r", b"deep").unwrap();
        writer.put_file("m/n", b"nested").unwrap();
        // A path below a file, at any depth, and a file where a directory is.
        assert_eq!(writer.put_file("a/b", b"x"), refused);
        assert_eq!(writer.put_file("a/b/c", b"x"), refused);
        assert_eq!(writer.put_file("m/n/o/p", b"x"), refused);
        assert_eq!(writer.put_file("p", b"x"), refused);
        assert_eq!(writer.put_file("p/q", b"x"), refused);
        // A name that only starts with another's is no directory of it, and a path may be
        // written again.
        writer.put_file("ab", b"ab").unwrap();
        writer.put_file("a.b", b"a.b").unwrap();
        writer.put_file("p/q/r", b"again").unwrap();
        writer.put_file("p/q/s", b"sibling").unwrap();
    }
    assert_eq!(
        store.files(&key).unwrap(),
        ["a", "a.b", "ab", "m/n", "p/q/r", "p/q/s"]
    );
    assert_eq!(store.get(&key, "a"), Ok(Some(b"file".to_vec())));
    assert_eq!(store.get(&key, "p/q/r"), Ok(Some(b"again".to_vec())));
    assert_eq!(store.get(&key, "p"), Ok(None));
    // A discard frees the names.
    store.discard(&key).unwrap();
    {
        let mut writer = store.begin(&key).unwrap();
        writer.put_file("a/b", b"now a directory").unwrap();
        assert_eq!(writer.put_file("a", b"x"), refused);
    }
    assert_eq!(store.files(&key).unwrap(), ["a/b"]);
}

#[test]
fn an_entry_reads_back_what_was_written_and_lists_sorted() {
    let mut store = FakeStore::new();
    let key = member("g1", "image");
    {
        let mut writer = store.begin(&key).unwrap();
        writer.put_file("image/oci-layout", b"layout").unwrap();
        writer.put_file("image/blobs/sha256/aa", b"blob").unwrap();
        writer.put_file("image/index.json", b"index").unwrap();
    }
    assert_eq!(
        store.files(&key).unwrap(),
        [
            "image/blobs/sha256/aa",
            "image/index.json",
            "image/oci-layout"
        ]
    );
    assert_eq!(
        store.get(&key, "image/oci-layout"),
        Ok(Some(b"layout".to_vec()))
    );
    assert_eq!(store.get(&key, "image/missing"), Ok(None));
    assert_eq!(store.get(&member("g1", "other"), "x"), Ok(None));
}

#[test]
fn a_second_begin_on_one_key_is_refused_and_a_discard_frees_it() {
    let mut store = FakeStore::new();
    let key = member("g1", "tool");
    store.begin(&key).unwrap().put_file("tool", b"1").unwrap();
    assert!(matches!(
        store.begin(&key),
        Err(SeamError::Refused(Reason::SetMember))
    ));
    assert_eq!(store.get(&key, "tool"), Ok(Some(b"1".to_vec())));
    store.discard(&key).unwrap();
    assert_eq!(store.files(&key), Ok(Vec::new()));
    store.begin(&key).unwrap().put_file("tool", b"2").unwrap();
    assert_eq!(store.get(&key, "tool"), Ok(Some(b"2".to_vec())));
}

#[test]
fn two_entries_do_not_touch_each_other_and_keys_are_listed_apart() {
    let mut store = FakeStore::new();
    let (a, b) = (member("g1", "tool"), member("g2", "tool"));
    let set = StoreKey::set(&scope("g1")).unwrap();
    store.begin(&a).unwrap().put_file("tool", b"a").unwrap();
    store.begin(&b).unwrap().put_file("tool", b"b").unwrap();
    store.begin(&set).unwrap().put_file("set.pb", b"s").unwrap();
    assert_eq!(store.get(&a, "tool"), Ok(Some(b"a".to_vec())));
    assert_eq!(store.get(&b, "tool"), Ok(Some(b"b".to_vec())));
    store.discard(&a).unwrap();
    assert_eq!(store.get(&b, "tool"), Ok(Some(b"b".to_vec())));
    assert_eq!(store.keys().unwrap(), [set, b]);
}
