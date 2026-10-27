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

fn scope_on(stage: &str, platform: &str, revision: &str, generation: &str) -> SetScope {
    SetScope {
        stage: stage.to_owned(),
        platform: platform.to_owned(),
        revision: revision.to_owned(),
        generation: generation.to_owned(),
    }
}

fn scope(stage: &str, generation: &str) -> SetScope {
    scope_on(stage, "linux-x86_64", "abc", generation)
}

fn key(stage: &str, generation: &str, member: &str) -> StoreKey {
    must(
        StoreKey::member(&scope(stage, generation), member),
        "a store key",
    )
}

/// The key of a member of a set that differs from `key("build", "g1", member)` in the platform
/// or the revision alone.
fn key_on(platform: &str, revision: &str, member: &str) -> StoreKey {
    must(
        StoreKey::member(&scope_on("build", platform, revision, "g1"), member),
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
    // blobs and deeper. A path written twice holds what the second write gave it, and nothing
    // of the first, however much longer it was, or when the second is empty, and at any depth.
    // A path written empty holds what a later write gives it.
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
        must(
            writer.put_file("rewritten", b"the first bytes"),
            "the first put_file",
        );
        must(writer.put_file("rewritten", b"two"), "the second put_file");
        must(
            writer.put_file("rewritten_empty", b"the first bytes"),
            "the first put_file of a path rewritten empty",
        );
        must(
            writer.put_file("rewritten_empty", b""),
            "the second put_file of a path rewritten empty",
        );
        must(
            writer.put_file("dir/empty", b""),
            "put_file of an empty file at a nested path",
        );
        must(
            writer.put_file("dir/rewritten", b"the first nested bytes"),
            "the first put_file of a nested path",
        );
        must(
            writer.put_file("dir/rewritten", b"two"),
            "the second put_file of a nested path",
        );
        must(
            writer.put_file("dir/rewritten_empty", b"the first nested bytes"),
            "the first put_file of a nested path rewritten empty",
        );
        must(
            writer.put_file("dir/rewritten_empty", b""),
            "the second put_file of a nested path rewritten empty",
        );
        must(
            writer.put_file("empty_then_bytes", b""),
            "put_file of a path written empty first",
        );
        must(
            writer.put_file("empty_then_bytes", b"later"),
            "put_file of bytes over a path written empty",
        );
        must(
            writer.put_file("dir/empty_then_bytes", b""),
            "put_file of a nested path written empty first",
        );
        must(
            writer.put_file("dir/empty_then_bytes", b"later"),
            "put_file of bytes over a nested path written empty",
        );
    }
    for (rel, want) in [
        ("binary", binary),
        ("big", big.as_slice()),
        ("empty", b"".as_slice()),
        ("blobs/sha256/abc", b"blob".as_slice()),
        ("a/b/c/d/e/f/g", b"deep".as_slice()),
        ("rewritten", b"two".as_slice()),
        ("rewritten_empty", b"".as_slice()),
        ("dir/empty", b"".as_slice()),
        ("dir/rewritten", b"two".as_slice()),
        ("dir/rewritten_empty", b"".as_slice()),
        ("empty_then_bytes", b"later".as_slice()),
        ("dir/empty_then_bytes", b"later".as_slice()),
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
            "dir/empty",
            "dir/empty_then_bytes",
            "dir/inner",
            "dir/rewritten",
            "dir/rewritten_empty",
            "empty",
            "empty_then_bytes",
            "rewritten",
            "rewritten_empty",
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
    // The order is the bytewise one: digits count as characters, and a character above U+FFFF
    // comes after U+FF5E, as it does in UTF-8 and not in UTF-16.
    let mut names: Vec<String> = ["z", "a/b", "m", "X", "x", "a", "a ", ".hidden", ".d/e"]
        .map(str::to_owned)
        .to_vec();
    names.extend(["e\u{301}", "\u{e9}", "\u{fc}"].map(str::to_owned));
    names.extend(["f1", "f2", "f10", "\u{1f600}", "\u{ff5e}"].map(str::to_owned));
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
    // A path the entry does not hold is nothing, though a path that differs from it only in
    // case, in a trailing space or in composition is held.
    let lone = key("build", "g1", "lone");
    {
        let mut writer = must(store.begin(&lone), "begin");
        must(writer.put_file("x", b"x"), "put_file");
        must(writer.put_file("e\u{301}", b"e"), "put_file");
    }
    for absent in ["X", "x ", "\u{e9}", "e", "y"] {
        same(
            must(store.get(&lone, absent), "get"),
            None,
            &format!("the path {absent:?}, which the entry does not hold"),
        );
    }
    same(
        must(store.files(&lone), "files"),
        vec!["e\u{301}".to_owned(), "x".to_owned()],
        "the paths of the entry that holds two",
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
    // Scopes that share a member name and differ in the stage, the platform, the revision or the
    // generation alone, a member whose name starts with another's, member names that differ only
    // in case, in a trailing space, in composition or in a character that is not ASCII, the key
    // of the set document beside the key of a member called `set`, and more keys than a store
    // might list at most.
    let mut made = vec![
        key("build", "g1", "tool"),
        key("build", "g1", "tool2"),
        key("build", "g2", "tool"),
        key("test", "g1", "tool"),
        key_on("linux-aarch64", "abc", "tool"),
        key_on("linux-x86_64", "def", "tool"),
        key("build", "g1", "Tool"),
        key("build", "g1", "tool "),
        key("build", "g1", "e\u{301}"),
        key("build", "g1", "\u{e9}"),
        key("build", "g1", "\u{fc}"),
        key("build", "g1", "set"),
        must(
            StoreKey::set(&scope("build", "g1")),
            "the key of a set document",
        ),
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
    // Files that a refused path may name once its climb, its dot parts or its slashes are
    // taken away, each with bytes of its own.
    let safe = ["x", "b", "c", "abs", "d"];
    {
        let mut writer = must(store.begin(&k), "begin");
        for rel in safe {
            must(
                writer.put_file(rel, format!("safe {rel}").as_bytes()),
                "put_file",
            );
        }
        for bad in [
            "", ".", "..", "../x", "a/../b", "./c", "/abs", "a//b", "a/", "a\\b", "a\nb", "a\0b",
            "a\tb", "a\rb", "a\u{7f}b", "a/..", "a/b/..", "a/.", "a/./b",
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
    let mut want: Vec<String> = safe.iter().map(|rel| (*rel).to_owned()).collect();
    want.push("y".to_owned());
    want.sort();
    same(
        files,
        want,
        "the files after refused paths: only the good ones",
    );
    // A refused path changes nothing: the files it might have named hold their own bytes.
    for rel in safe {
        same(
            must(store.get(&k, rel), "get"),
            Some(format!("safe {rel}").into_bytes()),
            &format!("the bytes of {rel} after refused paths"),
        );
    }
    same(
        must(store.get(&k, "y"), "get"),
        Some(b"after".to_vec()),
        "the file written after refused paths",
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
    // An entry whose key starts with another's holds files of its own: the shorter key does
    // not answer for them.
    let longer = key("build", "g1", "tool2");
    must(
        must(store.begin(&longer), "begin").put_file("only_in_the_longer", b"gamma"),
        "put_file",
    );
    same(
        must(store.get(&one, "only_in_the_longer"), "get"),
        None,
        "a path that only the entry of a longer key holds",
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
    // Entries with the member name of the discarded one, in another generation, stage,
    // platform or revision, are other entries too.
    let same_member = [
        key("build", "g2", "image"),
        key("test", "g1", "image"),
        key_on("linux-aarch64", "abc", "image"),
        key_on("linux-x86_64", "def", "image"),
    ];
    for (n, other) in same_member.iter().enumerate() {
        must(
            must(store.begin(other), "begin").put_file(&format!("s{n}"), b"same member"),
            "put_file",
        );
    }
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
    want.extend(same_member.iter().cloned());
    want.sort();
    same(
        kept,
        want.clone(),
        "the keys after the discard: the other entries alone, the one it is a prefix of and the ones with its member name too",
    );
    for (n, other) in same_member.iter().enumerate() {
        same(
            must(store.get(other, &format!("s{n}")), "get"),
            Some(b"same member".to_vec()),
            &format!("the entry {other} after the discard of another with its member name"),
        );
    }
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
    // An entry that never was is discarded without a fault and without a change: the entry
    // begun last, the others and the keys stay as they were, and the key is no entry after it.
    let latest = key("build", "g1", "latest");
    must(
        must(store.begin(&latest), "begin").put_file("last", b"latest"),
        "put_file",
    );
    want.push(latest.clone());
    want.sort();
    let never = [
        key("build", "g1", "never"),
        key("build", "g9", "image"),
        key_on("linux-aarch64", "abc", "never"),
    ];
    for missing in &never {
        must(
            store.discard(missing),
            &format!("a discard of {missing}, never begun"),
        );
        let mut now = must(store.keys(), "keys");
        now.sort();
        same(
            now,
            want.clone(),
            &format!("the keys after the discard of {missing}, which never was"),
        );
        same(
            must(store.get(&latest, "last"), "get"),
            Some(b"latest".to_vec()),
            "the entry begun last after the discard of one that never was",
        );
        same(
            must(store.get(&sibling, "o"), "get"),
            Some(b"keep".to_vec()),
            "the other entry after the discard of one that never was",
        );
    }
    drop(must(
        store.begin(&never[0]),
        "a begin on a key whose discard came before any begin",
    ));
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
