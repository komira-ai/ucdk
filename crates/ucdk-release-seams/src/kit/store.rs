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
    // and an empty file are entries like any other.
    let binary: &[u8] = &[0xff, 0xfe, 0x00, 0x80, 0xc3, 0x28];
    let big: Vec<u8> = (0..=255u8).cycle().take(10_000).collect();
    {
        let mut writer = must(store.begin(&k), "begin");
        must(writer.put_file("tool", b"bytes"), "put_file");
        must(
            writer.put_file("dir/inner", b"inner"),
            "put_file of a nested path",
        );
        must(writer.put_file("binary", binary), "put_file of bytes");
        must(writer.put_file("big", &big), "put_file of a large file");
        must(writer.put_file("empty", b""), "put_file of an empty file");
    }
    for (rel, want) in [
        ("binary", binary),
        ("big", big.as_slice()),
        ("empty", b"".as_slice()),
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
        ["big", "binary", "dir/inner", "empty", "tool"]
            .map(str::to_owned)
            .to_vec(),
        "the paths of the entry, the empty file among them",
    );
    same(
        must(store.get(&k, "dir"), "get"),
        None,
        "a directory of the entry is no file",
    );
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
    {
        let mut writer = must(store.begin(&k), "begin");
        for rel in ["z", "a/b", "m"] {
            must(writer.put_file(rel, rel.as_bytes()), "put_file");
        }
    }
    same(
        must(store.files(&k), "files"),
        vec!["a/b".to_owned(), "m".to_owned(), "z".to_owned()],
        "the paths of the entry, sorted",
    );
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
}

fn keys_of_two_scopes_are_listed_apart<R: StoreRig>(rig: &R) {
    let mut store = rig.store();
    let made = [
        key("build", "g1", "tool"),
        key("build", "g2", "tool"),
        key("test", "g1", "tool"),
    ];
    for (n, k) in made.iter().enumerate() {
        must(
            must(store.begin(k), "begin").put_file("f", &[n as u8]),
            "put_file",
        );
    }
    let mut listed = must(store.keys(), "keys");
    listed.sort();
    let mut want = made.to_vec();
    want.sort();
    same(
        listed,
        want,
        "the keys of three scopes that share a member name",
    );
    for k in &made {
        same(
            must(store.files(k), "files"),
            vec!["f".to_owned()],
            "the files of each entry",
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
    }
    same(
        must(store.files(&k), "files"),
        vec!["x".to_owned()],
        "the files after refused paths: only the good one",
    );
    // A path nobody wrote is nothing, however it climbs or spells the file that is there.
    for climbing in ["../x", "a/../x", "./x", "/x"] {
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
    let paths = ["a", "b/c", "d"];
    {
        let mut writer = must(store.begin(&k), "begin");
        for rel in paths {
            must(writer.put_file(rel, b"x"), "put_file");
        }
    }
    must(store.discard(&k), "discard");
    for rel in paths {
        same(
            must(store.get(&k, rel), "get"),
            None,
            &format!("{rel} after the discard"),
        );
    }
    same(
        must(store.keys(), "keys"),
        Vec::new(),
        "the keys after the discard",
    );
    must(store.begin(&k), "a begin on a key that was discarded");
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
