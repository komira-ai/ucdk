// SPDX-License-Identifier: Apache-2.0
//! The text of a value, as the file writes it and as anything that prints a value must.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use common::{Scratch, path, put};
use ucdk_core::{Locator, Tag};
use ucdk_ledger::{Change, FileLedger, Origin, Record, State, escape_token};

#[test]
fn escape_token_is_the_function_the_file_writer_uses() {
    // A record whose name, physical id and output value hold a newline, a space and an empty
    // text: every token of its lines in the file is `escape_token` of the value.
    let record = Record {
        origin: Origin::Created,
        state: State::Confirmed,
        physical: Some(Locator::new("id\nwith a space")),
        deps: BTreeSet::from([path("db/run")]),
        outputs: BTreeMap::from([
            ("odd key".to_owned(), "v\nrecord x/y".to_owned()),
            ("empty".to_owned(), String::new()),
        ]),
        ..Record::intent(
            path("app/run"),
            Tag::new("a kind"),
            "name\nwith a space",
            BTreeSet::new(),
        )
    };
    let scratch = Scratch::new("escape_token");
    put(
        &FileLedger::open(scratch.file()).expect("open"),
        Change::Put(record.clone()),
    );
    let text = fs::read_to_string(scratch.file()).expect("the file");
    let lines: BTreeSet<&str> = text.lines().collect();
    let has = |line: String| {
        assert!(
            lines.contains(line.as_str()),
            "{line} is not a line of {text}"
        );
    };
    has(format!("record {}", escape_token(record.path.as_str())));
    has(format!("kind {}", escape_token(record.kind.as_str())));
    has(format!("name {}", escape_token(&record.name)));
    has(format!(
        "physical {}",
        escape_token(record.physical.as_ref().expect("a physical id").as_str())
    ));
    has(format!("dep {}", escape_token("db/run")));
    for (key, value) in &record.outputs {
        has(format!(
            "output {} {}",
            escape_token(key),
            escape_token(value)
        ));
    }
    // And the spelling itself: what is kept, what is encoded, what the empty text becomes.
    assert_eq!(
        escape_token("name\nwith a space"),
        "name%0Awith%20a%20space"
    );
    assert_eq!(escape_token(""), "%");
    assert_eq!(escape_token("Az09._-/:"), "Az09._-/:");
    assert_eq!(escape_token("a=b,c%"), "a%3Db%2Cc%25");
    assert_eq!(escape_token("\u{e9}"), "%C3%A9");
}
