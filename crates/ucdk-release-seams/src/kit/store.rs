// SPDX-License-Identifier: Apache-2.0
//! The artifact store kit: eight scenarios over the keyed tree a set is written into.

use ucdk_release::store::{SetScope, StoreKey};

use super::{Report, Scenario, must, must_err, run_scenarios, same};
use crate::{ArtifactStore, SeamError};

/// What an implementation provides: an empty store each time.
pub trait StoreRig {
    type Store: ArtifactStore;

    fn store(&self) -> Self::Store;
}

fn scope(stage: &str, generation: &str) -> SetScope {
    SetScope {
        stage: stage.to_owned(),
        platform: "linux-x86_64".to_owned(),
        revision: "abc".to_owned(),
        generation: generation.to_owned(),
    }
}

fn key(stage: &str, generation: &str, member: &str) -> StoreKey {
    must(
        StoreKey::member(&scope(stage, generation), member),
        "a store key",
    )
}

pub fn run_store_kit<R: StoreRig>(rig: &R) -> Report {
    run_scenarios(rig, &store_scenarios())
}

pub fn store_scenarios<R: StoreRig>() -> Vec<Scenario<R>> {
    vec![
        Scenario::new(
            "an_entry_reads_back_what_was_written",
            an_entry_reads_back_what_was_written::<R>,
        ),
        Scenario::new(
            "an_entry_is_listed_exactly",
            an_entry_is_listed_exactly::<R>,
        ),
        Scenario::new(
            "a_second_begin_on_one_key_is_refused",
            a_second_begin_on_one_key_is_refused::<R>,
        ),
        Scenario::new(
            "keys_of_two_scopes_are_listed_apart",
            keys_of_two_scopes_are_listed_apart::<R>,
        ),
        Scenario::new(
            "paths_that_escape_an_entry_are_refused",
            paths_that_escape_an_entry_are_refused::<R>,
        ),
        Scenario::new(
            "two_entries_do_not_touch_each_other",
            two_entries_do_not_touch_each_other::<R>,
        ),
        Scenario::new(
            "discard_removes_every_path_of_an_entry",
            discard_removes_every_path_of_an_entry::<R>,
        ),
        Scenario::new(
            "a_missing_entry_lists_as_empty",
            a_missing_entry_lists_as_empty::<R>,
        ),
    ]
}

fn an_entry_reads_back_what_was_written<R: StoreRig>(rig: &R) {
    let mut store = rig.store();
    let k = key("build", "g1", "tool");
    // Bytes that are not text, a file past any buffer a store might size at a few kilobytes
    // and an empty file are entries like any other, and so are paths as deep as an OCI layout's
    // blobs and deeper. A path written twice holds what the second write gave it.
    let binary: &[u8] = &[0xff, 0xfe, 0x00, 0x80, 0xc3, 0x28];
    let big: Vec<u8> = (0..=255u8).cycle().take(100_000).collect();
    {
        let mut writer = must(store.begin(&k), "begin");
        must(writer.put_file("tool", b"bytes"), "put_file");
        must(
            writer.put_file("dir/inner", b"inner"),
            "put_file of a nested path",
        );
        must(
            writer.put_file("blobs/sha256/abc", b"blob"),
            "put_file of a path three parts deep",
        );
        must(
            writer.put_file("a/b/c/d/e/f/g", b"deep"),
            "put_file of a path seven parts deep",
        );
        must(writer.put_file("binary", binary), "put_file of bytes");
        must(writer.put_file("big", &big), "put_file of a large file");
        must(writer.put_file("empty", b""), "put_file of an empty file");
        must(writer.put_file("rewritten", b"one"), "the first put_file");
        must(writer.put_file("rewritten", b"two"), "the second put_file");
    }
    for (rel, want) in [
        ("binary", binary),
        ("big", big.as_slice()),
        ("empty", b"".as_slice()),
        ("blobs/sha256/abc", b"blob".as_slice()),
        ("a/b/c/d/e/f/g", b"deep".as_slice()),
        ("rewritten", b"two".as_slice()),
    ] {
        same(
            must(store.get(&k, rel), "get"),
            Some(want.to_vec()),
            &format!("the bytes of {rel}"),
        );
    }
    // The order is the business of `an_entry_is_listed_exactly`.
    let mut listed = must(store.files(&k), "files");
    listed.sort();
    same(
        listed,
        [
            "a/b/c/d/e/f/g",
            "big",
            "binary",
            "blobs/sha256/abc",
            "dir/inner",
            "empty",
            "rewritten",
            "tool",
        ]
        .map(str::to_owned)
        .to_vec(),
        "the paths of the entry, the empty file and the deep ones among them",
    );
    for directory in ["dir", "blobs/sha256"] {
        same(
            must(store.get(&k, directory), "get"),
            None,
            &format!("{directory}, a directory of the entry, is no file"),
        );
    }
    same(
        must(store.get(&k, "tool"), "get"),
        Some(b"bytes".to_vec()),
        "a written file",
    );
    same(
        must(store.get(&k, "dir/inner"), "get"),
        Some(b"inner".to_vec()),
        "a nested file",
    );
    same(
        must(store.get(&k, "missing"), "get"),
        None,
        "a path nobody wrote",
    );
}

fn an_entry_is_listed_exactly<R: StoreRig>(rig: &R) {
    let mut store = rig.store();
    let k = key("build", "g1", "image");
    // Paths that differ only in case, in a trailing space or in the composition of a character
    // are different paths, a dotfile is a file, and more than a hundred paths are all listed.
    let mut names: Vec<String> = ["z", "a/b", "m", "X", "x", "a", "a ", ".hidden", ".d/e"]
        .map(str::to_owned)
        .to_vec();
    names.extend(["e\u{301}", "\u{e9}"].map(str::to_owned));
    names.extend((0..120).map(|n| format!("many/f{n:03}")));
    {
        let mut writer = must(store.begin(&k), "begin");
        for rel in &names {
            must(writer.put_file(rel, rel.as_bytes()), "put_file");
        }
    }
    let mut want = names.clone();
    want.sort();
    same(
        must(store.files(&k), "files"),
        want,
        "the paths of the entry, sorted bytewise",
    );
    // What a read of a nested path does is the business of `an_entry_reads_back_what_was_written`.
    for rel in names.iter().filter(|rel| !rel.contains('/')) {
        same(
            must(store.get(&k, rel), "get"),
            Some(rel.as_bytes().to_vec()),
            &format!("the file {rel:?}, which another path may differ from only a little"),
        );
    }
}

fn a_second_begin_on_one_key_is_refused<R: StoreRig>(rig: &R) {
    let mut store = rig.store();
    let k = key("build", "g1", "tool");
    must(
        must(store.begin(&k), "begin").put_file("x", b"first"),
        "put_file",
    );
    let refusal = must_err(store.begin(&k), "a second begin");
    same(
        matches!(refusal, SeamError::Refused(_)),
        true,
        "the second begin is Refused",
    );
    same(
        must(store.get(&k, "x"), "get"),
        Some(b"first".to_vec()),
        "the entry after the refused begin",
    );
    // An entry that was begun and holds nothing is an entry: it is begun once.
    let empty = key("build", "g1", "empty");
    drop(must(store.begin(&empty), "begin of an entry left empty"));
    let refusal = must_err(store.begin(&empty), "a second begin of an empty entry");
    same(
        matches!(refusal, SeamError::Refused(_)),
        true,
        "the second begin of an entry that holds nothing is Refused",
    );
    let mut listed = must(store.keys(), "keys");
    listed.sort();
    let mut want = vec![k, empty];
    want.sort();
    same(listed, want, "the keys: an entry that holds nothing is one");
}

fn keys_of_two_scopes_are_listed_apart<R: StoreRig>(rig: &R) {
    let mut store = rig.store();
    // Three scopes that share a member name, a member whose name starts with another's, and
    // more keys than a store might list at most.
    let mut made = vec![
        key("build", "g1", "tool"),
        key("build", "g1", "tool2"),
        key("build", "g2", "tool"),
        key("test", "g1", "tool"),
    ];
    made.extend((0..120).map(|n| key("build", "g3", &format!("m{n:03}"))));
    for (n, k) in made.iter().enumerate() {
        must(
            must(store.begin(k), "begin").put_file(&format!("f{n}"), &[n as u8]),
            "put_file",
        );
    }
    let mut listed = must(store.keys(), "keys");
    listed.sort();
    let mut want = made.clone();
    want.sort();
    same(listed, want, "the keys of the scopes and members");
    for (n, k) in made.iter().enumerate() {
        same(
            must(store.files(k), "files"),
            vec![format!("f{n}")],
            &format!("the files of {k}, and no file of another entry"),
        );
    }
}

fn paths_that_escape_an_entry_are_refused<R: StoreRig>(rig: &R) {
    let mut store = rig.store();
    let k = key("build", "g1", "tool");
    {
        let mut writer = must(store.begin(&k), "begin");
        must(writer.put_file("x", b"safe"), "put_file");
        for bad in [
            "", ".", "..", "../x", "a/../b", "./c", "/abs", "a//b", "a/", "a\\b", "a\nb", "a\0b",
            "a\tb",
        ] {
            let refusal = must_err(writer.put_file(bad, b"x"), &format!("put_file of {bad:?}"));
            same(
                matches!(refusal, SeamError::Refused(_)),
                true,
                &format!("the path {bad:?} is Refused"),
            );
        }
        // A refused path does not spoil the writer: the next good one is written.
        must(
            writer.put_file("y", b"after"),
            "put_file of a good path after refused ones",
        );
    }
    let mut files = must(store.files(&k), "files");
    files.sort();
    same(
        files,
        vec!["x".to_owned(), "y".to_owned()],
        "the files after refused paths: only the good ones",
    );
    // A path nobody wrote is nothing, however it climbs or spells the file that is there.
    for climbing in ["../x", "a/../x", "./x", "/x", "x/"] {
        same(
            matches!(store.get(&k, climbing), Ok(None) | Err(_)),
            true,
            &format!("get of {climbing:?} reads no file"),
        );
    }
    same(
        must(store.keys(), "keys"),
        vec![k],
        "the keys after only refused paths",
    );
}

fn two_entries_do_not_touch_each_other<R: StoreRig>(rig: &R) {
    let mut store = rig.store();
    let (one, two) = (key("build", "g1", "tool"), key("build", "g2", "tool"));
    must(
        must(store.begin(&one), "begin").put_file("x", b"alpha"),
        "put_file",
    );
    must(
        must(store.begin(&two), "begin").put_file("x", b"beta"),
        "put_file",
    );
    same(
        must(store.get(&one, "x"), "get"),
        Some(b"alpha".to_vec()),
        "the first entry",
    );
    same(
        must(store.get(&two, "x"), "get"),
        Some(b"beta".to_vec()),
        "the second entry",
    );
    same(
        must(store.files(&one), "files"),
        vec!["x".to_owned()],
        "the files of the first entry",
    );
}

fn discard_removes_every_path_of_an_entry<R: StoreRig>(rig: &R) {
    let mut store = rig.store();
    let k = key("build", "g1", "image");
    let sibling = key("build", "g1", "other");
    // A key that the discarded one is a prefix of is another entry, begun before the
    // discarded one so that a begin must not mind it.
    let longer = key("build", "g1", "image2");
    let paths = ["a", "b/c", "d", "blobs/sha256/abc", "x/y/z/w"];
    must(
        must(store.begin(&longer), "begin").put_file("l", b"longer"),
        "put_file",
    );
    {
        let mut writer = must(store.begin(&k), "begin of a key a longer one starts with");
        for rel in paths {
            must(writer.put_file(rel, b"x"), "put_file");
        }
    }
    must(
        must(store.begin(&sibling), "begin").put_file("o", b"keep"),
        "put_file",
    );
    must(store.discard(&k), "discard");
    for rel in paths {
        same(
            must(store.get(&k, rel), "get"),
            None,
            &format!("{rel} after the discard"),
        );
    }
    let mut kept = must(store.keys(), "keys");
    kept.sort();
    let mut want = vec![sibling.clone(), longer.clone()];
    want.sort();
    same(
        kept,
        want,
        "the keys after the discard: the other entries alone, the one it is a prefix of too",
    );
    same(
        must(store.get(&sibling, "o"), "get"),
        Some(b"keep".to_vec()),
        "the other entry after the discard",
    );
    same(
        must(store.get(&longer, "l"), "get"),
        Some(b"longer".to_vec()),
        "the entry whose key the discarded one is a prefix of",
    );
    drop(must(store.begin(&k), "a begin on a key that was discarded"));
    // An entry that is gone, or never was, is discarded without a fault.
    must(store.discard(&k), "a discard of the entry just begun");
    must(store.discard(&k), "a discard of an entry that is gone");
    must(
        store.discard(&key("build", "g1", "never")),
        "a discard of an entry never begun",
    );
}

fn a_missing_entry_lists_as_empty<R: StoreRig>(rig: &R) {
    let mut store = rig.store();
    let k = key("build", "g1", "never");
    same(
        must(store.files(&k), "files"),
        Vec::new(),
        "the files of an entry never begun",
    );
    same(
        must(store.get(&k, "x"), "get"),
        None,
        "a path of an entry never begun",
    );
    same(
        must(store.keys(), "keys"),
        Vec::new(),
        "the keys of an empty store",
    );
}
